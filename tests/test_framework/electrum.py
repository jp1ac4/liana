import logging
import os

from ephemeral_port_reserve import reserve
from test_framework.utils import BitcoinBackend, TailableProc, ELECTRUM_PATH


class Electrum(BitcoinBackend):
    def __init__(
        self,
        bitcoind_dir,
        bitcoind_rpcport,
        bitcoind_p2pport,
        electrum_dir,
        rpcport=None,
    ):
        TailableProc.__init__(self, electrum_dir, verbose=False)

        if rpcport is None:
            rpcport = reserve()

        self.electrum_dir = electrum_dir
        self.rpcport = rpcport

        regtestdir = os.path.join(electrum_dir, "regtest")
        if not os.path.exists(regtestdir):
            os.makedirs(regtestdir)

        self.cmd_line = [
            ELECTRUM_PATH,
            "--conf",
            "{}/electrs.toml".format(regtestdir),
        ]
        electrum_conf = {
            "daemon_dir": bitcoind_dir,
            "cookie_file": os.path.join(bitcoind_dir, "regtest", ".cookie"),
            "daemon_rpc_addr": f"127.0.0.1:{bitcoind_rpcport}",
            "daemon_p2p_addr": f"127.0.0.1:{bitcoind_p2pport}",
            "db_dir": electrum_dir,
            "network": "regtest",
            "electrum_rpc_addr": f"127.0.0.1:{self.rpcport}",
        }
        self.conf_file = os.path.join(regtestdir, "electrs.toml")
        with open(self.conf_file, "w") as f:
            for k, v in electrum_conf.items():
                f.write(f'{k} = "{v}"\n')

    def start(self):
        TailableProc.start(self)
        logging.info("Electrum started")

    def startup(self):
        try:
            self.start()
        except Exception:
            self.stop()
            raise

    def stop(self):
        return TailableProc.stop(self)

    def cleanup(self):
        try:
            self.stop()
        except Exception:
            self.proc.kill()
        self.proc.wait()

    def append_to_conf(self, conf_file):
        with open(conf_file, "a") as f:
            f.write("[electrum_config]\n")
            f.write(f"addr = '127.0.0.1:{self.rpcport}'\n")
