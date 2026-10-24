"""Synthetic Python edge tests: no corpus environment or external bytes."""
import ast
import builtins
import importlib.resources
import json
import struct

import pytest
import swankit as ws


def cartridge():
    data = bytearray(131072)
    data[-16:] = bytes([0xEA, 0, 0, 0, 0xF0, 0, 1, 1, 7, 2, 0, 1, 0x0C, 0, 0, 0])
    data[-2:] = (sum(data[:-2]) & 0xFFFF).to_bytes(2, "little")
    return bytes(data)


def test_facts_and_detection():
    data = cartridge()
    facts = ws.rom_facts(data)
    assert isinstance(facts, ws.RomFacts)
    assert facts.size_bytes == 131072 and facts.color and facts.checksum_ok
    assert facts.game_id == 7 and facts.publisher_id == 1
    assert facts.declared_rom_bytes == 131072 and facts.save_bytes == 32768
    assert facts.to_dict()["rom_speed_cycles"] == 2
    detected = ws.detect(data, "wsc")
    assert detected.selected == "ws-rom"
    assert "ws-rom" in detected.candidates
    assert detected.confidence and detected.to_dict()["selected"] == "ws-rom"
    assert ws.detect(b"").selected is None
    with pytest.raises(AttributeError):
        facts.size_bytes = 3
    with pytest.raises(ws.SwankitError, match="not a recognized WonderSwan"):
        ws.rom_facts(b"bad")


@pytest.mark.parametrize("codec", ["lz-bits-16", "lz-rle-7f", "okumura-lzss"])
@pytest.mark.parametrize("raw", [b"", b"a", b"abracadabra" * 20, bytes(range(256)) * 2])
def test_codec_streams(codec, raw):
    encoded = ws.encode(codec, raw)
    assert isinstance(encoded, bytes)
    kwargs = {"decoded_size": len(raw)} if codec == "okumura-lzss" else {}
    decoded = ws.decode(codec, encoded + b"carrier", **kwargs)
    assert isinstance(decoded, ws.DecodeResult)
    assert decoded.data == raw and decoded.consumed == len(encoded)


@pytest.mark.parametrize("order", ["lsb-first", "msb-first"])
def test_okumura_policy(order):
    raw = b" " * 400
    encoded = ws.encode("okumura-lzss", raw, bit_order=order, fill=32)
    assert ws.decode("okumura-lzss", encoded, decoded_size=len(raw), bit_order=order, fill=32).data == raw
    with pytest.raises(ws.SwankitError, match="requires decoded_size"):
        ws.decode("okumura-lzss", encoded)
    with pytest.raises(ws.SwankitError, match="not external history"):
        ws.decode("okumura-lzss", encoded, decoded_size=len(raw), history=b"x")


def test_codec_errors_and_bounds():
    with pytest.raises(ws.SwankitError, match="unknown codec"):
        ws.encode("unknown", b"")
    with pytest.raises(ws.SwankitError, match="bit_order"):
        ws.decode("lz-bits-16", b"", bit_order="unknown")
    with pytest.raises(ws.SwankitError, match="65536"):
        ws.decode("lz-bits-16", b"", max_output=65537)
    with pytest.raises(ws.SwankitError, match="65536"):
        ws.encode("lz-rle-7f", bytes(65537))
    with pytest.raises(ws.ResourceLimitError, match="decoded bytes"):
        ws.decode("lz-rle-7f", ws.encode("lz-rle-7f", b"hello"), max_output=4)
    with pytest.raises(OverflowError):
        ws.decode("lz-rle-7f", b"", max_output=-1)
    for codec in ("lz-bits-16", "lz-rle-7f"):
        encoded = ws.encode(codec, b"token boundary test")
        for cut in range(len(encoded)):
            with pytest.raises(ws.SwankitError):
                ws.decode(codec, encoded[:cut])


@pytest.mark.parametrize("format,size", [("2bpp", 16), ("4bpp-planar", 32), ("4bpp-packed", 32)])
def test_tiles(format, size):
    image = ws.tiles(bytes(size), format, columns=1, scale=2)
    assert isinstance(image, ws.Image)
    assert (image.width, image.height) == (16, 16)
    assert image.rgba == bytes([255, 255, 255, 255]) * 256
    assert ws.to_pil(image).tobytes() == image.rgba
    source = bytes([0x80, 0]) + bytes(size - 2)
    if format == "4bpp-packed":
        source = bytes([0x10]) + bytes(size - 1)
    pixels = ws.tiles(source, format, columns=1).rgba
    assert pixels[:4] != pixels[4:8]


def test_palette_and_arrangement():
    palette = bytearray(512)
    palette[:2] = b"\x23\x01"
    assert ws.color_palette(bytes(palette))[:4] == bytes([17, 34, 51, 255])
    assert ws.mono_palette(bytes(256)) == bytes([255, 255, 255, 255]) * 4
    assert len(ws.grayscale(4)) == 16
    image = ws.tiles(bytes(64), columns=1, tiles_wide=2, tiles_high=2, column_major=True)
    assert (image.width, image.height) == (16, 16)
    for call in (
        lambda: ws.tiles(b""), lambda: ws.tiles(bytes(15)),
        lambda: ws.tiles(bytes(16), columns=0),
        lambda: ws.tiles(bytes(16), palette=b""),
        lambda: ws.tiles(bytes(16), format="invalid"),
        lambda: ws.color_palette(bytes(512), 16),
        lambda: ws.color_palette(bytes(513)), lambda: ws.mono_palette(bytes(20)),
        lambda: ws.grayscale(3),
    ):
        with pytest.raises(ws.SwankitError):
            call()
    with pytest.raises(ws.ResourceLimitError):
        ws.tiles(bytes(16), columns=2**32 - 1, scale=2**32 - 1)


def test_optional_pillow(monkeypatch):
    original = builtins.__import__
    def importing(name, *args, **kwargs):
        if name == "PIL":
            raise ImportError("Pillow absent")
        return original(name, *args, **kwargs)
    monkeypatch.setattr(builtins, "__import__", importing)
    with pytest.raises(ImportError, match="Pillow absent"):
        ws.to_pil(ws.tiles(bytes(16), columns=1))
    assert ws.tiles(bytes(16), columns=1).width == 8


def text_definition():
    return ws.TextDefinition(json.dumps({
        "schema": "swankit-text-def/1", "id": "synthetic",
        "map": {"schema": "swankit-charmap/1", "entries": [
            {"bytes": "01", "text": "A"}, {"bytes": "02", "text": "B"}],
            "controls": [{"code": 0, "ends": True, "display": ""}]},
        "tables": {"labels": {"kind": "offsets", "offsets": [0, 3], "maximum_bytes": 3, "require_end": True}},
        "fonts": {"small": {"offset": 0, "count": 1, "format": "bpp1", "stride": 8, "tiles_wide": 1, "tiles_high": 1}},
    }))


def test_text_roundtrip_and_dump():
    definition = text_definition()
    assert definition.id == "synthetic" and definition.to_dict()["schema"] == "swankit-text-def/1"
    decoded = definition.decode(b"\x01\x02\0padding")
    assert decoded.text == decoded.normalized == "AB"
    assert decoded.consumed == 3 and decoded.terminated and decoded.final_page == 0
    assert definition.encode("AB") == definition.encode_tokens(decoded) == b"\x01\x02\0"
    assert decoded.to_dict()["tokens"][0] == {"bytes": [1], "text": "A"}
    dump = definition.dump(b"\x01\x02\0\x02\0", "labels")
    assert [s["text"] for s in dump["strings"]] == ["AB", "B"]
    assert definition.font(bytes(8), "small", columns=1).rgba == bytes([255]*4) * 64
    assert ws.TextDefinition.builtin("word-dialog-1").id
    for call in (lambda: definition.decode(b"\x03"), lambda: definition.encode("C"),
                 lambda: definition.dump(b"", "absent"), lambda: ws.TextDefinition("[]")):
        with pytest.raises(ws.SwankitError):
            call()
    with pytest.raises(ws.ResourceLimitError):
        definition.encode("AB", max_bytes=1)


def test_sound():
    wave = ws.Wave(bytes([0xF0] * 16))
    assert wave.samples == bytes([0, 15] * 16)
    pcm = wave.to_pcm()
    assert isinstance(pcm, ws.Pcm)
    assert (pcm.channels, pcm.sample_rate, pcm.frame_count) == (1, 24000, 32)
    assert pcm.samples[:2] == [-32768, 28672]
    assert pcm.s16le()[:4] == struct.pack("<hh", -32768, 28672)
    assert len(ws.waves(bytes(32))) == 2
    pcm = ws.unsigned_pcm(bytes([0, 128, 255]), 8000)
    assert pcm.samples == [-32768, 0, 32512]
    assert pcm.sample_rate == 8000
    preview = ws.pcm_preview(bytes([0xFF] * 64), [(0, 0x88, 0xFF), (0, 0x90, 1), (0, 0x91, 8)], 5)
    assert preview.channels == 2 and preview.frame_count == 5
    assert len(preview.s16le()) == 20 and any(preview.samples)
    assert ws.pcm_preview(bytes(64), [], 4, coupled=True).samples == [0] * 8
    result = ws.sound_extract(bytes(16), json.dumps({"schema":"swankit-sound-def/1","format":"wave","length":16}))
    assert result["waves"] == [[0] * 32]
    for call in (lambda: ws.Wave(bytes(15)), lambda: ws.waves(bytes(17)),
                 lambda: wave.to_pcm(0), lambda: ws.unsigned_pcm(b"", 0),
                 lambda: ws.pcm_preview(bytes(64), [(3, 0x90, 1), (2, 0x90, 1)], 5)):
        with pytest.raises(ws.SwankitError):
            call()
    with pytest.raises(ws.ResourceLimitError):
        ws.pcm_preview(bytes(64), [], 24000 * 600 + 1)


def save_definition():
    const = lambda value: {"t": "Constant", "value": value}
    data = {"t": "Name", "id": "data"}
    magic = {"t": "Subscript", "value": data, "slice": const(0)}
    setting = {"t": "Subscript", "value": data, "slice": const(1)}
    def function(name, args, body):
        return {"t": "FunctionDef", "name": name, "args": {"args": [{"arg": n} for n in args], "defaults": []}, "body": body}
    return ws.SaveDefinition(json.dumps({
        "schema": "swankit-save-def/1", "id": "synthetic", "implementation": "generated",
        "media": [{"kind": "cartridge-eeprom", "sizes": [128]}],
        "fields": [{"name": "setting", "medium": "cart EEPROM", "offset": 1, "size": 1, "encoding": "u8"}],
        "records": [], "blocks": [], "checksums": {}, "primitives": {},
        "validity": {"policy": "program", "exclude": []},
        "entry": {"mode": "direct", "decode": "decode_image", "fix": "fix_image"},
        "program": {"t": "Module", "body": [
            function("main", [], [{"t": "Return", "value": const(None)}]),
            function("decode_image", ["data", "name"], [{"t": "Return", "value": {"t": "Dict", "keys": [const("valid"), const("setting")], "values": [{"t": "Compare", "left": magic, "ops": [{"t": "Eq"}], "comparators": [const(165)]}, setting]}}]),
            function("fix_image", ["data"], [{"t": "Assign", "targets": [magic], "value": const(165)}, {"t": "Return", "value": data}]),
        ], "type_ignores": []},
    }))


def test_save_transaction_and_write_back(tmp_path):
    definition = save_definition()
    original = bytes([0, 12]) + bytes([0x5A] * 126)
    assert definition.accepts_size(128) and not definition.accepts_size(127)
    assert definition.id == "synthetic" and definition.to_dict()["schema"] == "swankit-save-def/1"
    assert definition.fields[0]["name"] == "setting"
    assert definition.decode(original) == {"valid": False, "setting": 12}
    assert definition.verify(original) is None
    context = ws.SaveContext('{"schema":"swankit-save-context/1","tables":{},"images":{}}')
    fixed = definition.fix(original, context=context)
    assert fixed == b"\xA5" + original[1:]
    assert definition.verify(fixed)["usable_flags"] == 1
    changed = definition.set_field(original, 0, 7)
    assert changed == b"\xA5\x07" + original[2:]
    assert definition.set_field(original, 0, 8, repair=False) == b"\0\x08" + original[2:]
    path = tmp_path / "image.dat"
    path.write_bytes(changed)
    assert definition.decode(path.read_bytes(), name=path.name) == {"valid": True, "setting": 7}
    assert original[0] == 0
    assert ws.detect(fixed, "eep", context=ws.SaveContext()).selected == "ws-save-image"
    for call in (lambda: definition.set_field(original, 0, 256),
                 lambda: definition.set_field(original, 2**63, 0),
                 lambda: definition.fix(bytes(127)), lambda: ws.SaveDefinition("[]"),
                 lambda: ws.SaveContext("{}")):
        with pytest.raises(ws.SwankitError):
            call()
    ids = ws.save_definitions()
    assert ids and ws.SaveDefinition.builtin(ids[0]).id == ids[0]


def test_save_v2_constraints_and_skip_validation_are_explicit():
    definition = save_definition().to_dict()
    definition["schema"] = "swankit-save-def/2"
    definition["fields"][0]["type"] = "unsigned"
    definition["fields"][0]["constraint"] = {"kind": "unsigned-range", "minimum": 0, "maximum": 9}
    definition = ws.SaveDefinition(json.dumps(definition))
    original = bytes([165, 1]) + bytes(126)
    with pytest.raises(ws.SwankitError):
        definition.set_field(original, 0, 10, repair=False)
    changed = definition.set_field(original, 0, 10, repair=False, skip_validation=True)
    assert changed == bytes([165, 10]) + bytes(126)
    with pytest.raises(ws.SwankitError):
        definition.set_field(original, 0, 256, repair=False, skip_validation=True)


@pytest.mark.parametrize("target", [b"abc", b"ab", b"abcdef", b"aaaaaa", b""])
def test_ips_roundtrip(target):
    base = b"abc"
    patch = ws.ips_create(base, target)
    assert ws.detect(patch, "ips").selected == "ws-patch-ips"
    assert ws.ips_apply(base, patch, allow_growth=True) == target
    if len(target) > len(base):
        with pytest.raises(ws.SwankitError, match="grow"):
            ws.ips_apply(base, patch)


def test_ips_overlap_and_rust_error_text():
    patch = b"PATCH" + b"\0\0\0\0\1A" + b"\0\0\0\0\1B" + b"EOF"
    with pytest.raises(ws.SwankitError, match="overlapping"):
        ws.ips_apply(b"x", patch)
    assert ws.ips_apply(b"x", patch, allow_overlap=True) == b"B"
    with pytest.raises(ws.ResourceLimitError, match="IPS output bytes"):
        ws.ips_apply(b"x", patch, allow_overlap=True, max_output=0)
    with pytest.raises(ws.SwankitError) as caught:
        ws.ips_apply(b"", b"wrong")
    assert str(caught.value) == "IPS header is not PATCH"


def test_stubs_and_public_surface_are_packaged():
    package = importlib.resources.files("swankit")
    assert package.joinpath("py.typed").is_file()
    tree = ast.parse(package.joinpath("__init__.pyi").read_text())
    names = {node.name for node in tree.body if isinstance(node, (ast.ClassDef, ast.FunctionDef))}
    assert names == set(ws.__all__)


def test_unsupported_and_contextual_errors():
    with pytest.raises(ws.UnsupportedError, match="IPS creation target exceeds 24-bit size"):
        ws.ips_create(b"", bytes(0x1000001))
    with pytest.raises(ws.SwankitError) as caught:
        ws.ips_apply(b"", b"PATCH\0\0\0")
    assert str(caught.value) == "record 0: unexpected end of input: needed 2 byte(s) at offset 8, 0 available"


@pytest.mark.parametrize("codec", ["lz-bits-16", "lz-rle-7f", "okumura-lzss"])
def test_codec_resource_subclasses(codec):
    with pytest.raises(ws.ResourceLimitError, match="65536") as caught:
        ws.encode(codec, bytes(65537))
    assert isinstance(caught.value, ws.SwankitError)
    for kwargs in ({"max_output": 65537}, {"history": bytes(65537)}):
        with pytest.raises(ws.ResourceLimitError, match="65536"):
            ws.decode(codec, b"", **kwargs)
    # Malformed selectors/options remain distinct from resource exhaustion.
    for call in (lambda: ws.encode("unknown", b""),
                 lambda: ws.encode(codec, b"", bit_order="invalid")):
        with pytest.raises(ws.SwankitError) as malformed:
            call()
        assert not isinstance(malformed.value, ws.ResourceLimitError)
    raw = b"bounded valid input" * 4
    encoded = ws.encode(codec, raw)
    kwargs = {"decoded_size": len(raw)} if codec == "okumura-lzss" else {}
    assert ws.decode(codec, encoded, **kwargs).data == raw
