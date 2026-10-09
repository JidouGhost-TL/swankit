"""Line-based transport for the Rust corpus harness; all reads use the extension."""
import hashlib
import json
import os
import sys

import swankit as ws

assert os.environ.get("FORMATKIT_CORPUS_RUNNER_LOCKED") == "1"
assert os.environ.get("FORMATKIT_CORPUS_REQUIRED") == "1"
assert os.environ.get("FORMATKIT_CORPUS_WRITE", "0") == "0"


def digest(data, args):
    sha256 = lambda value: hashlib.sha256(value).hexdigest()
    if args["operation"] == "detect":
        detected = ws.detect(data)
        if args["kind"] == "ws-rom":
            info = ws.rom_facts(data).to_dict()
            reference = args["reference"]
            for key in ("size_bytes", "color", "checksum_ok", "orientation", "publisher"):
                assert (info[key] if info[key] is not None else "?") == reference[key], key
            for key in ("checksum_computed", "checksum_footer"):
                assert f"{info[key]:04x}" == reference[key], key
        return {"selected": detected.selected, "sha256": sha256(data)}
    if args["operation"] == "codec":
        codec = args["codec"]
        decoded = ws.decode(codec, data, max_output=args["raw_len"])
        encoded = ws.encode(codec, decoded.data)
        back = ws.decode(codec, encoded, max_output=args["raw_len"])
        assert back.data == decoded.data and back.consumed == len(encoded)
        fits = len(encoded) <= len(data)
        if args["require_fit"]:
            assert fits
        return {"codec": codec, "raw_len": len(decoded.data), "stored_len": len(data),
                "raw_sha256": sha256(decoded.data), "stored_sha256": sha256(data),
                "consumed": decoded.consumed, "encoded_len": len(encoded),
                "encoded_sha256": sha256(encoded), "fits_original": fits}
    if args["operation"] == "save":
        definition = ws.SaveDefinition.builtin(args["id"])
        context = ws.SaveContext(json.dumps(args["context"]))
        name = args["name"]
        result = {"selected": ws.detect(data).selected, "sha256": sha256(data),
                  "decoded": definition.decode(data, name=name, context=context),
                  "matched": definition.verify(data, name=name, context=context) is not None}
        if "corrupt_offset" in args:
            corrupted = bytearray(data)
            corrupted[args["corrupt_offset"]] ^= 1
            fixed = definition.fix(bytes(corrupted), name=name, context=context)
            result.update(repaired=definition.decode(fixed, name=name, context=context),
                          repaired_sha256=sha256(fixed))
        return result
    raise ValueError("unknown binding corpus operation")


for line in sys.stdin:
    request = json.loads(line)
    if request is None:
        break
    try:
        response = digest(bytes(request["data"]), request["args"])
    except Exception as error:
        response = {"error": str(error)}
    print(json.dumps(response), flush=True)
