use crate::database::sqlite::{FreshDbOptions, SqliteDbError, DB_VERSION};

use std::{convert::TryInto, fs, path, time};

use miniscript::bitcoin::{self, secp256k1};
use rusqlite::Transaction;

pub const LOOK_AHEAD_LIMIT: u32 = 200;

/// Perform a set of modifications to the database inside a single transaction
pub fn db_exec<F>(conn: &mut rusqlite::Connection, modifications: F) -> Result<(), rusqlite::Error>
where
    F: FnOnce(&rusqlite::Transaction) -> rusqlite::Result<()>,
{
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    modifications(&tx)?;
    tx.commit()
}

/// Internal helper for queries boilerplate
pub fn db_tx_query<P, F, T>(
    tx: &rusqlite::Transaction,
    stmt_str: &str,
    params: P,
    f: F,
) -> Result<Vec<T>, rusqlite::Error>
where
    P: IntoIterator + rusqlite::Params,
    P::Item: rusqlite::ToSql,
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    tx.prepare(stmt_str)?
        .query_map(params, f)?
        .collect::<rusqlite::Result<Vec<T>>>()
}

/// Internal helper for queries boilerplate
pub fn db_query<P, F, T>(
    conn: &mut rusqlite::Connection,
    stmt_str: &str,
    params: P,
    f: F,
) -> Result<Vec<T>, rusqlite::Error>
where
    P: IntoIterator + rusqlite::Params,
    P::Item: rusqlite::ToSql,
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    conn.prepare(stmt_str)?
        .query_map(params, f)?
        .collect::<rusqlite::Result<Vec<T>>>()
}

/// The current time as the number of seconds since the UNIX epoch, truncated to u32 since SQLite
/// only supports i64 integers.
pub fn curr_timestamp() -> u32 {
    time::SystemTime::now()
        .duration_since(time::UNIX_EPOCH)
        .expect("System clock went backward the epoch?")
        .as_secs()
        .try_into()
        .expect("Is this the year 2106 yet? Misconfigured system clock.")
}

// Create the db file with RW permissions only for the user
pub fn create_db_file(db_path: &path::Path) -> Result<(), std::io::Error> {
    let mut options = fs::OpenOptions::new();
    let options = options.read(true).write(true).create_new(true);

    #[cfg(unix)]
    return {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(0o600).open(db_path)?;
        Ok(())
    };

    #[cfg(not(unix))]
    return {
        // TODO: permissions for Windows...
        options.open(db_path)?;
        Ok(())
    };
}

/// Create a fresh Liana database with the given schema.
pub fn create_fresh_db(
    db_path: &path::Path,
    options: FreshDbOptions,
    secp: &secp256k1::Secp256k1<secp256k1::VerifyOnly>,
) -> Result<(), SqliteDbError> {
    create_db_file(db_path)?;

    let timestamp = curr_timestamp();

    // Fill the initial addresses. On a fresh database, the deposit_derivation_index is
    // necessarily 0.
    let mut query = String::with_capacity(100 * LOOK_AHEAD_LIMIT as usize);
    for index in 0..LOOK_AHEAD_LIMIT {
        let receive_address = options
            .main_descriptor
            .receive_descriptor()
            .derive(index.into(), secp)
            .address(options.bitcoind_network);
        let change_address = options
            .main_descriptor
            .change_descriptor()
            .derive(index.into(), secp)
            .address(options.bitcoind_network);
        query += &format!(
            "INSERT INTO addresses (receive_address, change_address, derivation_index) VALUES (\"{}\", \"{}\", {});\n",
            receive_address, change_address, index
        );
    }

    let mut conn = rusqlite::Connection::open(db_path)?;
    db_exec(&mut conn, |tx| {
        tx.execute_batch(options.schema)?;
        tx.execute(
            "INSERT INTO version (version) VALUES (?1)",
            rusqlite::params![options.version],
        )?;
        tx.execute(
            "INSERT INTO tip (network, blockheight, blockhash) VALUES (?1, NULL, NULL)",
            rusqlite::params![options.bitcoind_network.to_string()],
        )?;
        tx.execute(
            "INSERT INTO wallets (timestamp, main_descriptor, deposit_derivation_index, change_derivation_index) \
                     VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![timestamp, options.main_descriptor.to_string(), 0, 0],
        )?;
        tx.execute_batch(&query)?;

        Ok(())
    })?;

    Ok(())
}

pub fn db_version(conn: &mut rusqlite::Connection) -> Result<i64, SqliteDbError> {
    Ok(db_query(
        conn,
        "SELECT version FROM version",
        rusqlite::params![],
        |row| {
            let version: i64 = row.get(0)?;
            Ok(version)
        },
    )?
    .pop()
    .expect("There is always a row in the version table"))
}

/// Update `is_from_self` in coins table for all unconfirmed coins
/// and those confirmed after `current_tip_height`.
///
/// This only sets the value to true as we do not expect the value
/// to change from true to false. In case of a reorg, the value
/// for all unconfirmed coins should be set to false before this
/// function is called.
pub fn update_coins_from_self(
    db_tx: &Transaction,
    current_tip_height: i32,
) -> Result<(), rusqlite::Error> {
    // Given the requirement for unconfirmed coins that all ancestors
    // be from self, we perform the update in a loop until no further
    // rows are updated in order to iterate over the unconfirmed coins.
    let mut i = 0;
    loop {
        i += 1;
        // Although we don't expect any unconfirmed transaction to have
        // more than 25 in-mempool descendants including itself, there
        // could be more descendants in the DB following a reorg and a
        // rollback of the tip. Therefore, we can't be sure how many
        // iterations will be required here and can't set a max number
        // of iterations. However, the query only sets `is_from_self`
        // to 1 for those coins with value 0 and so the number of rows
        // affected by each iteration must become 0, given there are a
        // finite number of rows, at which point the loop will break.
        if i % 5 == 0 {
            log::debug!("Setting is_from_self. Starting iteration {}..", i);
        }
        let updated = db_tx.execute(
            "
            UPDATE coins
            SET is_from_self = 1
            FROM transactions t
                INNER JOIN (
                    SELECT
                        spend_txid,
                        SUM(
                            CASE
                                WHEN blockheight IS NOT NULL THEN 1
                                -- If the spending coin is unconfirmed, only count
                                -- it as an input coin if it is from self.
                                WHEN blockheight IS NULL AND is_from_self = 1 THEN 1
                                ELSE 0
                            END
                        ) AS cnt
                    FROM coins
                    WHERE spend_txid IS NOT NULL
                    -- We only need to consider spend transactions that are
                    -- unconfirmed or confirmed after `current_tip_height
                    -- as only these transactions will affect the coins that
                    -- we are updating.
                    AND (spend_block_height IS NULL OR spend_block_height > ?1)
                    GROUP BY spend_txid
                ) spends
                ON t.txid = spends.spend_txid AND t.num_inputs = spends.cnt
            WHERE coins.txid = t.txid
            AND (coins.blockheight IS NULL OR coins.blockheight > ?1)
            AND coins.is_from_self = 0
            ",
            [current_tip_height],
        )?;
        if updated == 0 {
            break Ok(());
        }
    }
}

// In Liana 0.4 we upgraded the schema to hold a timestamp for transaction drafts. Existing
// transaction drafts are not set any timestamp on purpose.
fn migrate_v0_to_v1(conn: &mut rusqlite::Connection) -> Result<(), SqliteDbError> {
    db_exec(conn, |tx| {
        tx.execute(
            "ALTER TABLE spend_transactions ADD COLUMN updated_at",
            rusqlite::params![],
        )?;
        tx.execute("UPDATE version SET version = 1", rusqlite::params![])?;
        Ok(())
    })?;

    Ok(())
}

// After Liana 1.0 we upgraded the schema to record whether a coin originated from an immature
// coinbase transaction.
fn migrate_v1_to_v2(conn: &mut rusqlite::Connection) -> Result<(), SqliteDbError> {
    db_exec(conn, |tx| {
        tx.execute(
            "ALTER TABLE coins ADD COLUMN is_immature BOOLEAN NOT NULL DEFAULT 0 CHECK (is_immature IN (0,1))",
            rusqlite::params![],
        )?;
        tx.execute("UPDATE version SET version = 2", rusqlite::params![])?;
        Ok(())
    })?;

    Ok(())
}

// After Liana 1.1 we upgraded the schema to add the labels table.
fn migrate_v2_to_v3(conn: &mut rusqlite::Connection) -> Result<(), SqliteDbError> {
    db_exec(conn, |tx| {
        tx.execute(
            "CREATE TABLE labels (id INTEGER PRIMARY KEY NOT NULL, wallet_id INTEGER NOT NULL, item_kind INTEGER NOT NULL CHECK (item_kind IN (0,1,2)), item TEXT UNIQUE NOT NULL, value TEXT NOT NULL)",
            rusqlite::params![],
        )?;
        tx.execute("UPDATE version SET version = 3", rusqlite::params![])?;
        Ok(())
    })?;

    Ok(())
}

fn migrate_v3_to_v4(conn: &mut rusqlite::Connection) -> Result<(), SqliteDbError> {
    db_exec(conn, |tx| {
        tx.execute_batch(
            "CREATE TABLE coins_new (
                id INTEGER PRIMARY KEY NOT NULL,
                wallet_id INTEGER NOT NULL,
                blockheight INTEGER,
                blocktime INTEGER,
                txid BLOB NOT NULL,
                vout INTEGER NOT NULL,
                amount_sat INTEGER NOT NULL,
                derivation_index INTEGER NOT NULL,
                is_change BOOLEAN NOT NULL CHECK (is_change IN (0,1)),
                spend_txid BLOB,
                spend_block_height INTEGER,
                spend_block_time INTEGER,
                is_immature BOOLEAN NOT NULL CHECK (is_immature IN (0,1)),
                UNIQUE (txid, vout),
                FOREIGN KEY (wallet_id) REFERENCES wallets (id)
                    ON UPDATE RESTRICT
                    ON DELETE RESTRICT
            );

            INSERT INTO coins_new SELECT * FROM coins;

            DROP TABLE coins;

            ALTER TABLE coins_new RENAME TO coins;

            UPDATE version SET version = 4;",
        )
    })?;
    Ok(())
}

fn migrate_v4_to_v5(
    conn: &mut rusqlite::Connection,
    bitcoin_txs: &[bitcoin::Transaction],
) -> Result<(), SqliteDbError> {
    db_exec(conn, |db_tx| {
        db_tx.execute(
            "
            CREATE TABLE transactions (
                id INTEGER PRIMARY KEY NOT NULL,
                txid BLOB UNIQUE NOT NULL,
                tx BLOB UNIQUE NOT NULL
            );",
            rusqlite::params![],
        )?;

        for bitcoin_tx in bitcoin_txs {
            let txid = &bitcoin_tx.txid()[..].to_vec();
            let bitcoin_tx_ser = bitcoin::consensus::serialize(bitcoin_tx);
            db_tx.execute(
                "INSERT INTO transactions (txid, tx) VALUES (?1, ?2);",
                rusqlite::params![txid, bitcoin_tx_ser,],
            )?;
        }

        // Create new coins table with foreign key constraints on transactions table.
        db_tx.execute_batch(
            "
            CREATE TABLE coins_new (
                id INTEGER PRIMARY KEY NOT NULL,
                wallet_id INTEGER NOT NULL,
                blockheight INTEGER,
                blocktime INTEGER,
                txid BLOB NOT NULL,
                vout INTEGER NOT NULL,
                amount_sat INTEGER NOT NULL,
                derivation_index INTEGER NOT NULL,
                is_change BOOLEAN NOT NULL CHECK (is_change IN (0,1)),
                spend_txid BLOB,
                spend_block_height INTEGER,
                spend_block_time INTEGER,
                is_immature BOOLEAN NOT NULL CHECK (is_immature IN (0,1)),
                UNIQUE (txid, vout),
                FOREIGN KEY (wallet_id) REFERENCES wallets (id)
                    ON UPDATE RESTRICT
                    ON DELETE RESTRICT,
                FOREIGN KEY (txid) REFERENCES transactions (txid)
                    ON UPDATE RESTRICT
                    ON DELETE RESTRICT,
                FOREIGN KEY (spend_txid) REFERENCES transactions (txid)
                    ON UPDATE RESTRICT
                    ON DELETE RESTRICT
            );

            INSERT INTO coins_new SELECT * FROM coins;

            DROP TABLE coins;

            ALTER TABLE coins_new RENAME TO coins;

            UPDATE version SET version = 5;",
        )
    })?;
    Ok(())
}

fn migrate_v5_to_v6(conn: &mut rusqlite::Connection) -> Result<(), SqliteDbError> {
    db_exec(conn, |tx| {
        tx.execute(
            "ALTER TABLE wallets ADD COLUMN last_poll_timestamp INTEGER",
            rusqlite::params![],
        )?;
        tx.execute("UPDATE version SET version = 6", rusqlite::params![])?;
        Ok(())
    })?;

    Ok(())
}

fn migrate_v6_to_v7(conn: &mut rusqlite::Connection) -> Result<(), SqliteDbError> {
    db_exec(conn, |db_tx| {
        let num_coins: i64 =
            db_tx.query_row("SELECT count(*) AS cnt FROM coins", [], |row| row.get(0))?;
        // Create a copy of the coins table without any FK constraints.
        db_tx.execute_batch(
            "
            CREATE TABLE coins_copy (
                id INTEGER PRIMARY KEY NOT NULL,
                wallet_id INTEGER NOT NULL,
                blockheight INTEGER,
                blocktime INTEGER,
                txid BLOB NOT NULL,
                vout INTEGER NOT NULL,
                amount_sat INTEGER NOT NULL,
                derivation_index INTEGER NOT NULL,
                is_change BOOLEAN NOT NULL CHECK (is_change IN (0,1)),
                spend_txid BLOB,
                spend_block_height INTEGER,
                spend_block_time INTEGER,
                is_immature BOOLEAN NOT NULL CHECK (is_immature IN (0,1)),
                UNIQUE (txid, vout)
            );

            INSERT INTO coins_copy (
                id,
                wallet_id,
                blockheight,
                blocktime,
                txid,
                vout,
                amount_sat,
                derivation_index,
                is_change,
                spend_txid,
                spend_block_height,
                spend_block_time,
                is_immature
            ) SELECT
                id,
                wallet_id,
                blockheight,
                blocktime,
                txid,
                vout,
                amount_sat,
                derivation_index,
                is_change,
                spend_txid,
                spend_block_height,
                spend_block_time,
                is_immature
            FROM coins;
            ",
        )?;

        // Make sure all rows of coins_copy match the expected values.
        let num_coins_intersect: i64 = db_tx.query_row(
            "SELECT count(*) AS cnt FROM
        (
            SELECT
                id,
                wallet_id,
                blockheight,
                blocktime,
                txid,
                vout,
                amount_sat,
                derivation_index,
                is_change,
                spend_txid,
                spend_block_height,
                spend_block_time,
                is_immature
            FROM
                coins

            INTERSECT

            SELECT
                id,
                wallet_id,
                blockheight,
                blocktime,
                txid,
                vout,
                amount_sat,
                derivation_index,
                is_change,
                spend_txid,
                spend_block_height,
                spend_block_time,
                is_immature
            FROM
                coins_copy
        )",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(num_coins, num_coins_intersect);

        let num_txs: i64 =
            db_tx.query_row("SELECT count(*) AS cnt FROM transactions", [], |row| {
                row.get(0)
            })?;
        // Rename transactions table and add the new `num_inputs` column as nullable.
        db_tx.execute_batch(
            "
            ALTER TABLE transactions RENAME TO transactions_old;
            ALTER TABLE transactions_old ADD COLUMN num_inputs INTEGER;
            ",
        )?;

        // Iterate over transactions, updating the `num_inputs` column.
        let txids = db_tx
            .prepare("SELECT txid FROM transactions_old")?
            .query_map([], |row| {
                let txid: Vec<u8> = row.get(0)?;
                let txid: bitcoin::Txid = bitcoin::consensus::encode::deserialize(&txid)
                    .expect("We only store valid txids");
                Ok(txid)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let num_txids: i64 = txids
            .len()
            .try_into()
            .expect("Number of txids must be less than 2**63");
        assert_eq!(num_txids, num_txs);
        for txid in &txids {
            let tx = db_tx.query_row(
                "SELECT tx FROM transactions_old WHERE txid = ?1",
                rusqlite::params![txid[..].to_vec()],
                |row| {
                    let tx: Vec<u8> = row.get(0)?;
                    let tx: bitcoin::Transaction = bitcoin::consensus::encode::deserialize(&tx)
                        .expect("We only store valid transactions");
                    Ok(tx)
                },
            )?;
            let num_inputs: i64 = tx
                .input
                .len()
                .try_into()
                .expect("Num tx inputs must be less than 2**63");
            db_tx.execute(
                "UPDATE transactions_old SET num_inputs = ?1 WHERE txid = ?2",
                rusqlite::params![num_inputs, txid[..].to_vec()],
            )?;
        }

        // Create the new transactions table and copy over the data.
        db_tx.execute_batch(
            "
            CREATE TABLE transactions (
                id INTEGER PRIMARY KEY NOT NULL,
                txid BLOB UNIQUE NOT NULL,
                tx BLOB UNIQUE NOT NULL,
                num_inputs INTEGER NOT NULL
            );

            INSERT INTO transactions(id, txid, tx, num_inputs)
            SELECT id, txid, tx, num_inputs
            FROM transactions_old;",
        )?;
        // Make sure we have the expected values.
        let num_txs_intersect: i64 = db_tx.query_row(
            "SELECT count(*) AS cnt FROM
            (
                SELECT id, txid, tx, num_inputs
                FROM transactions
                INTERSECT
                SELECT id, txid, tx, num_inputs
                FROM transactions_old
            )",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(num_txs, num_txs_intersect);

        // Create and populate the new coins table,
        // setting `is_from_self` to 0 for all rows.
        db_tx.execute_batch(
            "
            DROP TABLE coins;
            DROP TABLE transactions_old;

            CREATE TABLE coins (
                id INTEGER PRIMARY KEY NOT NULL,
                wallet_id INTEGER NOT NULL,
                blockheight INTEGER,
                blocktime INTEGER,
                txid BLOB NOT NULL,
                vout INTEGER NOT NULL,
                amount_sat INTEGER NOT NULL,
                derivation_index INTEGER NOT NULL,
                is_change BOOLEAN NOT NULL CHECK (is_change IN (0,1)),
                spend_txid BLOB,
                spend_block_height INTEGER,
                spend_block_time INTEGER,
                is_immature BOOLEAN NOT NULL CHECK (is_immature IN (0,1)),
                is_from_self BOOLEAN NOT NULL CHECK (is_from_self IN (0,1)),
                CHECK (is_immature IS 0 OR is_from_self IS 0),
                UNIQUE (txid, vout),
                FOREIGN KEY (wallet_id) REFERENCES wallets (id)
                    ON UPDATE RESTRICT
                    ON DELETE RESTRICT,
                FOREIGN KEY (txid) REFERENCES transactions (txid)
                    ON UPDATE RESTRICT
                    ON DELETE RESTRICT,
                FOREIGN KEY (spend_txid) REFERENCES transactions (txid)
                    ON UPDATE RESTRICT
                    ON DELETE RESTRICT
            );

            INSERT INTO coins (
                id,
                wallet_id,
                blockheight,
                blocktime,
                txid,
                vout,
                amount_sat,
                derivation_index,
                is_change,
                spend_txid,
                spend_block_height,
                spend_block_time,
                is_immature,
                is_from_self
            ) SELECT
                id,
                wallet_id,
                blockheight,
                blocktime,
                txid,
                vout,
                amount_sat,
                derivation_index,
                is_change,
                spend_txid,
                spend_block_height,
                spend_block_time,
                is_immature,
                0
            FROM coins_copy;
            ",
        )?;

        // Make sure all rows in the newly created coins table are as expected.
        let num_coins_intersect: i64 = db_tx.query_row(
            "SELECT count(*) AS cnt FROM
        (
            SELECT
                id,
                wallet_id,
                blockheight,
                blocktime,
                txid,
                vout,
                amount_sat,
                derivation_index,
                is_change,
                spend_txid,
                spend_block_height,
                spend_block_time,
                is_immature,
                is_from_self
            FROM
                coins

            INTERSECT

            SELECT
                id,
                wallet_id,
                blockheight,
                blocktime,
                txid,
                vout,
                amount_sat,
                derivation_index,
                is_change,
                spend_txid,
                spend_block_height,
                spend_block_time,
                is_immature,
                0 -- we set all rows to 0
            FROM
                coins_copy
        )",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(num_coins, num_coins_intersect);

        {
            // Make sure there are no coins with confirmation height <= 0:
            let row_count: i64 = db_tx.query_row(
                "SELECT count(*) AS cnt FROM coins WHERE blockheight <= 0",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(row_count, 0);
        }
        // Update the `is_from_self` column for all unconfirmed coins and those
        // confirmed after height 0, i.e. this will act on all coins.
        // For now, we use this utils function directly in the migration as we
        // don't expect any changes to the DB schema that would stop this function
        // working on a V6 DB schema. In future, this migration can use a "frozen"
        // version of this function if required.
        update_coins_from_self(db_tx, 0)?;

        db_tx.execute_batch(
            "
            DROP TABLE coins_copy;

            UPDATE version SET version = 7;",
        )?;

        Ok(())
    })?;
    Ok(())
}

/// Check the database version and if necessary apply the migrations to upgrade it to the current
/// one. The `bitcoin_txs` parameter is here for the migration from versions 4 and earlier, which
/// did not store the Bitcoin transactions in database, to versions 5 and later, which do. For a
/// migration from v4 or earlier to v5 or later it is assumed the caller passes *all* necessary
/// transactions, otherwise the migration will fail.
pub fn maybe_apply_migration(
    db_path: &path::Path,
    bitcoin_txs: &[bitcoin::Transaction],
) -> Result<(), SqliteDbError> {
    let mut conn = rusqlite::Connection::open(db_path)?;

    // Iteratively apply the database migrations necessary.
    loop {
        let version = db_version(&mut conn)?;
        match version {
            DB_VERSION => {
                log::info!("Database is up to date.");
                return Ok(());
            }
            0 => {
                log::warn!("Upgrading database from version 0 to version 1.");
                migrate_v0_to_v1(&mut conn)?;
                log::warn!("Migration from database version 0 to version 1 successful.");
            }
            1 => {
                log::warn!("Upgrading database from version 1 to version 2.");
                migrate_v1_to_v2(&mut conn)?;
                log::warn!("Migration from database version 1 to version 2 successful.");
            }
            2 => {
                log::warn!("Upgrading database from version 2 to version 3.");
                migrate_v2_to_v3(&mut conn)?;
                log::warn!("Migration from database version 2 to version 3 successful.");
            }
            3 => {
                log::warn!("Upgrading database from version 3 to version 4.");
                migrate_v3_to_v4(&mut conn)?;
                log::warn!("Migration from database version 3 to version 4 successful.");
            }
            4 => {
                log::warn!("Upgrading database from version 4 to version 5.");
                log::warn!(
                    "Number of bitcoin transactions to be inserted: {}.",
                    bitcoin_txs.len()
                );
                migrate_v4_to_v5(&mut conn, bitcoin_txs)?;
                log::warn!("Migration from database version 4 to version 5 successful.");
            }
            5 => {
                log::warn!("Upgrading database from version 5 to version 6.");
                migrate_v5_to_v6(&mut conn)?;
                log::warn!("Migration from database version 5 to version 6 successful.");
            }
            6 => {
                log::warn!("Upgrading database from version 6 to version 7.");
                migrate_v6_to_v7(&mut conn)?;
                log::warn!("Migration from database version 6 to version 7 successful.");
            }
            _ => return Err(SqliteDbError::UnsupportedVersion(version)),
        }
    }
}
