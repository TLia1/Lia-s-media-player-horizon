"""Regenerate tiny original decoder fixtures with Pillow (test-only dependency)."""
from pathlib import Path
from PIL import Image, ImageDraw, ImageSequence
root = Path(__file__).resolve().parents[1] / 'mixins/liasmediaplayer/lias_media_player/tests/fixtures'
root.mkdir(exist_ok=True)
im = Image.new('RGBA', (3, 2))
im.putdata([(255,0,0,255),(0,255,0,128),(0,0,255,0),(12,34,56,255),(255,255,255,255),(0,0,0,255)])
im.save(root/'rgba.png')
im.convert('RGB').save(root/'rgb.bmp')
im.save(root/'rgba.webp', lossless=True)
im.convert('RGB').save(root/'rgb.jpg', quality=95)
im.convert('L').save(root/'gray.jpg', quality=95)
(root/'rgba.raw').write_bytes(im.tobytes())
frames=[]
for i in range(4):
    frame=Image.new('RGBA',(8,6),(0,0,0,0))
    ImageDraw.Draw(frame).rectangle((i, i, i+3, i+2), fill=[(255,0,0,255),(0,255,0,255),(0,0,255,255),(255,255,0,255)][i])
    frames.append(frame)
frames[0].save(root/'disposal.gif', save_all=True, append_images=frames[1:], duration=[40,60,80,100], loop=0, disposal=[1,2,3,1], optimize=True)
frames[0].save(root/'animated.webp', save_all=True, append_images=frames[1:], duration=[40,60,80,100], loop=0, lossless=True)
# Dictionary growth + interlacing.
grad=Image.new('P',(64,48));grad.putpalette([v for i in range(256) for v in (i,(i*7)%256,(i*13)%256)])
grad.putdata([(x*17+y*31)%256 for y in range(48) for x in range(64)])
grad.save(root/'interlaced.gif',interlace=True)
for name in ['disposal.gif','interlaced.gif','animated.webp']:
    with Image.open(root/name) as image:
        (root/(name+'.raw')).write_bytes(b''.join(frame.convert('RGBA').tobytes() for frame in ImageSequence.Iterator(image)))

# Milestone 3: original APNG timing/RGBA oracle and a lossless AVIF still.
# FFmpeg with libaom-av1 is required only to regenerate the AVIF fixture.
import subprocess
a = Image.new('RGBA', (4, 3), (255, 0, 0, 255))
b = Image.new('RGBA', (4, 3), (0, 255, 0, 128))
c = Image.new('RGBA', (4, 3), (0, 0, 255, 255))
a.save(root/'animated.apng', save_all=True, append_images=[b, c],
       duration=[40, 120, 260], loop=0, disposal=[0, 0, 0], blend=[0, 0, 0])
(root/'animated.apng.raw').write_bytes(a.tobytes() + b.tobytes() + c.tobytes())
import io
buffer = io.BytesIO()
a.save(buffer, format='PNG')
subprocess.run(['ffmpeg', '-v', 'error', '-i', 'pipe:0', '-frames:v', '1',
                '-c:v', 'libaom-av1', '-still-picture', '1', '-crf', '0',
                '-y', str(root/'still.avif')], input=buffer.getvalue(), check=True)
