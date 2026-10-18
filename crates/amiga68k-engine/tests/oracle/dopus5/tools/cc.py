"""Tiny Copperline control client (library): c = CC('ccp.json'); c.call('status')."""
import json, socket
class CC:
    def __init__(s, info):
        info = json.load(open(info)); h, p = info["listen"].rsplit(":", 1)
        s.s = socket.create_connection((h, int(p))); s.f = s.s.makefile("rw"); s.n = 0
        s.call("hello", {"proto": 1}); a = s.call("auth", {"token": info["token"]})
        assert "error" not in a, a
    def call(s, m, p=None):
        s.n += 1
        s.f.write(json.dumps({"jsonrpc": "2.0", "id": s.n, "method": m, "params": p or {}}) + "\n"); s.f.flush()
        while True:
            r = json.loads(s.f.readline())
            if r.get("id") == s.n: return r.get("result", r)
