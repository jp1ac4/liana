# lianad integration tests

These tests can run against either bitcoind or Electrum Server backends, and with either legacy (P2WSH) or Taproot descriptors.

## Running Tests

### Single Backend

Run tests with the default backend (bitcoind):
```bash
cargo test -p lianad --tests
```

Run tests with Electrum backend:
```bash
LIANA_TEST_BACKEND=electrum cargo test -p lianad --tests
```

### All Backends

Run all tests against both backends:
```bash
./scripts/test_all_backends.sh
```

Run specific tests against both backends:
```bash
./scripts/test_all_backends.sh test_name_pattern
```

## Backend Selection

Tests automatically select the backend based on environment variables:

- `LIANA_TEST_BACKEND=electrum|electrs|electrumd` - Use Electrum Server
- `LIANA_TEST_BACKEND=bitcoind|bitcoind-rpc|rpc` - Use bitcoind RPC
- `LIANA_TEST_ELECTRUM=1|true` - Use Electrum Server (legacy)
- Default: bitcoind RPC

## Descriptor Selection

Tests can also select descriptor types:

- `LIANA_TEST_TAPROOT=1|true` - Use Taproot descriptors
- `LIANA_TEST_DESCRIPTOR=taproot|tr|tap` - Use Taproot descriptors
- Default: Legacy P2WSH descriptors

## Examples

```bash
# Legacy descriptors with bitcoind (default)
cargo test -p lianad --tests

# Taproot descriptors with bitcoind
LIANA_TEST_TAPROOT=1 cargo test -p lianad --tests

# Legacy descriptors with Electrum
LIANA_TEST_BACKEND=electrum cargo test -p lianad --tests

# Taproot descriptors with Electrum
LIANA_TEST_BACKEND=electrum LIANA_TEST_TAPROOT=1 cargo test -p lianad --tests
```

Note: When using Taproot, `lianad` enforces a minimum bitcoind version (see
`MIN_TAPROOT_BITCOIND_VERSION` in `lianad/src/bitcoin/d/mod.rs`). Our
CorePC-based test harness currently uses a recent regtest node via `electrsd`,
which satisfies this requirement.
