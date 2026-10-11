import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

cli, case = sys.argv[1:]
with tempfile.TemporaryDirectory(prefix="swankit-fifo-") as directory:
    root = Path(directory)
    fifo = root / "input.fifo"
    os.mkfifo(fifo)
    if case == "cli":
        result = subprocess.run([cli,"info",str(fifo)],capture_output=True,text=True,timeout=2)
        assert result.returncode != 0 and "regular file" in result.stderr, result
    else:
        payload = root / "payload.dat"
        payload.write_bytes(bytes(544))
        requests = "\n".join(json.dumps({"version":1,"operation":2,"path":str(path)}) for path in [fifo,payload])+"\n"
        result = subprocess.run([cli,"interop","--json","--server"],input=requests,capture_output=True,text=True,timeout=2)
        assert result.returncode == 0, result.stderr
        replies = [json.loads(line) for line in result.stdout.splitlines()]
        assert len(replies)==2 and "regular file" in replies[0]["error"] and len(replies[1]["output"])==256, replies
