"""utm-par2 (MOD-21): integrator excerpt of how the task reached each session, from the product's session files in a
run's collected.tar (withheld; listed in withheld-files.txt). Prints, per session file (by SHA-256), every user-role input
message other than the environment context (first 400 characters) and whether any model input (not the model's own
reasoning) contains the `corbanu_untrusted_data` wrapper.   usage: excerpt_tasks.py COLLECTED_TAR"""
import hashlib
import json
import sys
import tarfile

with tarfile.open(sys.argv[1]) as tar:
    for m in sorted(tar.getmembers(), key=lambda m: m.name):
        if not (m.isfile() and "/sessions/" in m.name and m.name.endswith(".jsonl")):
            continue
        data = tar.extractfile(m).read()
        print("== session file %s (sha256 %s)" % (m.name.rsplit("/", 1)[-1], hashlib.sha256(data).hexdigest()))
        wrapped = []
        for line in data.decode().splitlines():
            d = json.loads(line)
            p = d.get("payload", {})
            if p.get("type") == "session_meta":
                meta = p.get("payload", p)
                print("  session_meta source=%s" % json.dumps(meta.get("source"))[:200])
            if p.get("type") == "reasoning":
                continue
            s = json.dumps(p)
            if "corbanu_untrusted_data" in s:
                wrapped.append("%s %s/%s" % (d.get("timestamp"), p.get("type"), p.get("role")))
            if p.get("type") == "message" and p.get("role") == "user":
                text = " ".join(c.get("text", "") for c in p.get("content", []) if isinstance(c, dict))
                if text.lstrip().startswith("<environment_context>"):
                    continue
                print("  %s user message: %s" % (d.get("timestamp"), text[:400].replace("\n", " | ")))
        print("  model inputs containing corbanu_untrusted_data: %s" % (wrapped or "none"))
