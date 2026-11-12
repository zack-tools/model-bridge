#!/usr/bin/env python3
"""
ModelBridge Icon Generator
Generates:
  - Vector SVG icon (apps/desktop/src-tauri/icons/icon.svg and apps/desktop/ui/assets/icon.svg)
  - Master PNG 1024x1024, 512x512, 256x256, 128x128, 64x64, 32x32
  - Multi-resolution Windows ICO (icon.ico)
  - macOS ICNS (icon.icns) using iconutil
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path
from PIL import Image

WORKSPACE_ROOT = Path(__file__).resolve().parent.parent
TAURI_ICONS_DIR = WORKSPACE_ROOT / "apps" / "desktop" / "src-tauri" / "icons"
UI_ASSETS_DIR = WORKSPACE_ROOT / "apps" / "desktop" / "ui" / "assets"

def get_svg_content():
  return (WORKSPACE_ROOT / "scripts" / "icon-design.svg").read_text(encoding="utf-8")

def generate_icons():
    TAURI_ICONS_DIR.mkdir(parents=True, exist_ok=True)
    UI_ASSETS_DIR.mkdir(parents=True, exist_ok=True)

    svg_content = get_svg_content()
    svg_path = TAURI_ICONS_DIR / "icon.svg"
    ui_svg_path = UI_ASSETS_DIR / "icon.svg"

    svg_path.write_text(svg_content, encoding="utf-8")
    ui_svg_path.write_text(svg_content, encoding="utf-8")
    print(f"✓ Saved vector SVG to {svg_path} and {ui_svg_path}")

    # Render 1024x1024 master PNG with transparent background
    temp_dir = Path("/tmp/modelbridge_icon_build")
    temp_dir.mkdir(parents=True, exist_ok=True)
    temp_svg = temp_dir / "master.svg"
    temp_svg.write_text(svg_content, encoding="utf-8")
    rendered_png = temp_dir / "master_rendered.png"

    rsvg_bin = shutil.which("rsvg-convert")
    if not rsvg_bin:
        raise RuntimeError("rsvg-convert is required to render transparent icons")
    cmd = [rsvg_bin, "-w", "1024", "-h", "1024", str(temp_svg), "-o", str(rendered_png)]
    subprocess.run(cmd, check=True)

    master_img = Image.open(rendered_png).convert("RGBA")
    # Ensure exact 1024x1024
    if master_img.size != (1024, 1024):
        master_img = master_img.resize((1024, 1024), Image.Resampling.LANCZOS)

    # Save 1024x1024 master icon.png in tauri icons and ui assets
    master_png_path = TAURI_ICONS_DIR / "icon.png"
    master_img.save(master_png_path, "PNG")
    master_img.save(UI_ASSETS_DIR / "icon.png", "PNG")
    print(f"✓ Saved master 1024x1024 icon.png to {master_png_path}")

    # Standard Tauri & Desktop PNG sizes
    sizes = {
        "16x16.png": 16,
        "32x32.png": 32,
        "64x64.png": 64,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "256x256.png": 256,
        "256x256@2x.png": 512,
        "512x512.png": 512,
        "512x512@2x.png": 1024,
        "Square30x30Logo.png": 30,
        "Square44x44Logo.png": 44,
        "Square71x71Logo.png": 71,
        "Square89x89Logo.png": 89,
        "Square107x107Logo.png": 107,
        "Square142x142Logo.png": 142,
        "Square150x150Logo.png": 150,
        "Square284x284Logo.png": 284,
        "Square310x310Logo.png": 310,
        "StoreLogo.png": 50,
    }

    for filename, s in sizes.items():
        resized = master_img.resize((s, s), Image.Resampling.LANCZOS)
        resized.save(TAURI_ICONS_DIR / filename, "PNG")
    print(f"✓ Generated {len(sizes)} standard resolution PNGs in {TAURI_ICONS_DIR}")

    # Generate Windows icon.ico with multiple resolutions (16, 24, 32, 48, 64, 128, 256)
    ico_sizes = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    ico_path = TAURI_ICONS_DIR / "icon.ico"
    master_img.save(ico_path, format="ICO", sizes=ico_sizes)
    print(f"✓ Generated multi-size Windows icon.ico at {ico_path}")

    # Generate macOS icon.icns using iconutil
    iconset_dir = temp_dir / "ModelBridge.iconset"
    iconset_dir.mkdir(parents=True, exist_ok=True)
    mac_sizes = [
        ("icon_16x16.png", 16),
        ("icon_16x16@2x.png", 32),
        ("icon_32x32.png", 32),
        ("icon_32x32@2x.png", 64),
        ("icon_128x128.png", 128),
        ("icon_128x128@2x.png", 256),
        ("icon_256x256.png", 256),
        ("icon_256x256@2x.png", 512),
        ("icon_512x512.png", 512),
        ("icon_512x512@2x.png", 1024),
    ]
    for name, sz in mac_sizes:
        res_img = master_img.resize((sz, sz), Image.Resampling.LANCZOS)
        res_img.save(iconset_dir / name, "PNG")

    icns_path = TAURI_ICONS_DIR / "icon.icns"
    iconutil_cmd = ["iconutil", "-c", "icns", str(iconset_dir), "-o", str(icns_path)]
    iconutil_res = subprocess.run(iconutil_cmd, capture_output=True, text=True)
    if iconutil_res.returncode == 0 and icns_path.exists():
        print(f"✓ Generated native Apple macOS icon.icns at {icns_path}")
    else:
        print(f"Warning: iconutil failed ({iconutil_res.stderr}), skipping icns creation.")

if __name__ == "__main__":
    generate_icons()
