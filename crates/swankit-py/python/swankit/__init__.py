"""WonderSwan operations backed directly by Rust definitions and results."""
from ._native import (
    DecodeResult, Detection, Image, Pcm, ResourceLimitError, RomFacts,
    SaveContext, SaveDefinition, SwankitError, TextDefinition, TextResult,
    UnsupportedError, Wave, color_palette, decode, detect, encode, grayscale,
    ips_apply, ips_create, mono_palette, pcm_preview, rom_facts,
    save_definitions, sound_extract, tiles, unsigned_pcm, waves,
)

__all__ = [
    "DecodeResult", "Detection", "Image", "Pcm", "ResourceLimitError", "RomFacts",
    "SaveContext", "SaveDefinition", "SwankitError", "TextDefinition", "TextResult",
    "UnsupportedError", "Wave", "color_palette", "decode", "detect", "encode",
    "grayscale", "ips_apply", "ips_create", "mono_palette", "pcm_preview",
    "rom_facts", "save_definitions", "sound_extract", "tiles", "to_pil",
    "unsigned_pcm", "waves",
]


def to_pil(image: Image):
    """Convert an RGBA image to Pillow; Pillow is an optional dependency."""
    from PIL import Image as PillowImage
    return PillowImage.frombytes("RGBA", (image.width, image.height), image.rgba)
