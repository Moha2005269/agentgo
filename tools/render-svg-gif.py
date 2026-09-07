import os
import shutil
import subprocess
from playwright.sync_api import sync_playwright

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SVG_PATH = os.path.join(ROOT, "assets", "agentgo-loop.svg")
GIF_PATH = os.path.join(ROOT, "assets", "agentgo-loop.gif")
FRAMES_DIR = os.path.join(ROOT, "assets", ".frames_temp")

if os.path.exists(FRAMES_DIR):
    shutil.rmtree(FRAMES_DIR)
os.makedirs(FRAMES_DIR, exist_ok=True)

FPS = 20
DURATION = 3.6
TOTAL_FRAMES = int(FPS * DURATION)

print(f"Rendering {TOTAL_FRAMES} frames from {SVG_PATH}...")
with sync_playwright() as p:
    browser = p.chromium.launch(
        executable_path=r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        headless=True
    )
    page = browser.new_page(viewport={"width": 960, "height": 540}, device_scale_factor=1)
    page.goto(f"file:///{SVG_PATH}")
    page.evaluate("""() => {
        if (document.documentElement) document.documentElement.style.background = '#0b0d12';
        if (document.body) document.body.style.background = '#0b0d12';
        document.getAnimations().forEach(a => a.pause());
    }""")
    
    for i in range(TOTAL_FRAMES):
        t_ms = (i / FPS) * 1000
        page.evaluate(f"() => document.getAnimations().forEach(a => a.currentTime = {t_ms})")
        page.screenshot(path=os.path.join(FRAMES_DIR, f"frame_{i:04d}.png"))
        
    browser.close()

print("Assembling GIF via FFmpeg...")
subprocess.run([
    "ffmpeg", "-y", "-framerate", str(FPS),
    "-i", os.path.join(FRAMES_DIR, "frame_%04d.png"),
    "-vf", "split[s0][s1];[s0]palettegen=max_colors=128[p];[s1][p]paletteuse=dither=bayer:bayer_scale=3",
    GIF_PATH
], check=True)

# Generate preview
subprocess.run([
    "ffmpeg", "-y", "-ss", "00:00:02.5", "-i", GIF_PATH,
    "-vframes", "1", os.path.join(ROOT, "assets", "agentgo-loop-preview.png")
], check=True)

shutil.rmtree(FRAMES_DIR)
print(f"Successfully generated {GIF_PATH}")
