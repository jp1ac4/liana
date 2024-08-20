import hashlib
import logging
import os
import socket
import time

from decimal import Decimal
from ephemeral_port_reserve import reserve
from test_framework.authproxy import AuthServiceProxy
from test_framework.utils import TailableProc, wait_for, TIMEOUT, ELECTRUM_PATH, COIN


# class ElectrumRpcInterface:
#     def __init__(self, rpc_port):
#         self.rpc_port = rpc_port

#     def __getattr__(self, name):
#         assert not (name.startswith("__") and name.endswith("__")), "Python internals"

#         service_url = f"http://localhost:{self.rpc_port}"
#         proxy = AuthServiceProxy(service_url, name)

#         def f(*args):
#             return proxy.__call__(*args)

#         # Make debuggers show <function electrum.rpc.name> rather than <function
#         # electrum.rpc.<lambda>>
#         f.__name__ = name
#         return f


class Electrum(TailableProc):
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
        # self.p2pport = reserve()
        # self.prefix = "electrum"
        # self.bitcoin_rpcport = bitcoind.rpcport
        # self.bitcoin_p2pport = bitcoind.p2pport
        # self.bitcoin_dir = bitcoind.bitcoin_dir

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

        # self.rpc = ElectrumRpcInterface(rpcport)

    def start(self):
        # self.bitcoind.start()
        TailableProc.start(self)
        # self.wait_for_log("waiting", timeout=TIMEOUT)
        logging.info("Electrum started")

    def startup(self):
        try:
            self.start()
        except Exception:
            self.stop()
            raise

        # info = self.rpc.getnetworkinfo()
        # if info["version"] < 220000:
        #     self.rpc.stop()
        #     raise ValueError(
        #         "bitcoind is too old. Minimum supported version is 0.22.0."
        #         " Current is {}".format(info["version"])
        #     )

    def stop(self):
        return TailableProc.stop(self)

    def cleanup(self):
        try:
            self.stop()
        except Exception:
            self.proc.kill()
        self.proc.wait()
