#!/usr/bin/env python3
"""Builds the colour library (`*.clut`) from `palettes.json`.

Every palette is read from a Copernicus Sentinel-2 Level-2A true-colour image
(TCI), as `README.md` describes. The images come from the Sentinel-2
Cloud-Optimized GeoTIFFs on the AWS Registry of Open Data and are cached in
`--cache`.

    python3 build_palettes.py [--out DIR] [--cache DIR] [--sources FILE]

Needs Python 3.10 or newer with numpy, scipy, pillow and rasterio.
"""
import argparse, json, os, struct, sys
import numpy as np
from scipy.ndimage import gaussian_filter
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
COLOURS = 512

# sRGB (D65) <-> CIELAB.
M = np.array([[0.4124564, 0.3575761, 0.1804375],
              [0.2126729, 0.7151522, 0.0721750],
              [0.0193339, 0.1191920, 0.9503041]])
WHITE = np.array([0.95047, 1.0, 1.08883])


def srgb_to_linear(c):
    c = np.asarray(c, dtype=np.float64) / 255.0
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


LUT = srgb_to_linear(np.arange(256)).astype(np.float32)


def image_to_lab(rgb):
    """an 8-bit sRGB image in CIELAB (float32)"""
    xyz = LUT[rgb] @ (M.T / WHITE).astype(np.float32)
    d = np.float32(6 / 29)
    f = np.where(xyz > d ** 3, np.cbrt(xyz), xyz / (3 * d * d) + np.float32(4 / 29))
    return np.stack([116 * f[..., 1] - 16, 500 * (f[..., 0] - f[..., 1]), 200 * (f[..., 1] - f[..., 2])], -1).astype(np.float32)


def lab_to_rgb(lab):
    lab = np.asarray(lab, dtype=np.float64)
    fy = (lab[..., 0] + 16) / 116
    f = np.stack([fy + lab[..., 1] / 500, fy, fy - lab[..., 2] / 200], -1)
    d = 6 / 29
    xyz = np.where(f > d, f ** 3, 3 * d * d * (f - 4 / 29)) * WHITE
    c = np.clip(xyz @ np.linalg.inv(M).T, 0.0, 1.0)
    return 255.0 * np.where(c <= 0.0031308, 12.92 * c, 1.055 * np.power(c, 1 / 2.4) - 0.055)


# The blurs a path is read through: the largest at most half the distance
# between its samples.
SIGMAS = [0.0, 1.0, 2.0, 4.0]


def blurred(lab, sigma):
    if sigma == 0:
        return lab
    return np.stack([gaussian_filter(lab[..., k], sigma, mode='nearest') for k in range(3)], -1)


def sigma_for(spacing):
    return max(s for s in SIGMAS if s <= 0.5 * spacing)


def bilinear(img, x, y):
    h, w = img.shape[:2]
    x = np.clip(x, 0, w - 1.001)
    y = np.clip(y, 0, h - 1.001)
    x0 = np.floor(x).astype(np.int64)
    y0 = np.floor(y).astype(np.int64)
    fx = (x - x0)[..., None]
    fy = (y - y0)[..., None]
    top = img[y0, x0] * (1 - fx) + img[y0, x0 + 1] * fx
    bottom = img[y0 + 1, x0] * (1 - fx) + img[y0 + 1, x0 + 1] * fx
    return top * (1 - fy) + bottom * fy


MAX_BEND = 0.2
MAX_LOG_SPEED = float(np.log(2.0))


def path_points(path, bend, speeds, n=COLOURS):
    """The n points a palette is read at: a quadratic Bezier curve from the
    path's start to its end whose control point is `bend` chord lengths to
    the side of the chord's middle, walked at `speeds` (the log of the
    relative speed in each quarter of the palette)."""
    x0, y0, x1, y1 = path
    bend = float(np.clip(bend, -MAX_BEND, MAX_BEND))
    speed = np.exp(np.clip(np.asarray(speeds, dtype=np.float64), -MAX_LOG_SPEED, MAX_LOG_SPEED))
    knots = np.concatenate([[0.0], np.cumsum(speed / speed.sum())])
    t = np.interp((np.arange(n) + 0.5) / n, [0.0, 0.25, 0.5, 0.75, 1.0], knots)
    p0 = np.array([x0, y0], dtype=np.float64)
    p1 = np.array([x1, y1], dtype=np.float64)
    chord = p1 - p0
    length = np.hypot(*chord)
    normal = np.array([-chord[1], chord[0]]) / max(length, 1e-9)
    q = (p0 + p1) / 2 + bend * length * normal
    return ((1 - t) ** 2)[:, None] * p0 + (2 * t * (1 - t))[:, None] * q + (t ** 2)[:, None] * p1


def read_path(lab_levels, path, bend, speeds):
    """the CIELAB colours along a path, and the blur they were read through"""
    pts = path_points(path, bend, speeds)
    spacing = np.hypot(*np.diff(pts, axis=0).T).sum() / COLOURS
    sigma = sigma_for(spacing)
    return bilinear(lab_levels(sigma), pts[:, 0], pts[:, 1]).astype(np.float64), sigma, pts


def adjust(lab, contrast, lightness, saturation, white_balance):
    """lightness contrast about L* = 50 and offset; chroma scale and a shift
    of a* and b*"""
    out = np.empty_like(lab)
    out[..., 0] = 50.0 + contrast * (lab[..., 0] - 50.0) + lightness
    out[..., 1] = saturation * lab[..., 1] + white_balance[0]
    out[..., 2] = saturation * lab[..., 2] + white_balance[1]
    return out


def render(lab_levels, entry):
    raw, _, _ = read_path(lab_levels, entry['path'], entry['bend'], entry['speeds'])
    a = entry['adjust']
    lab = adjust(raw, a['contrast'], a['lightness'], a['saturation'], a['white_balance'])
    return np.clip(np.rint(lab_to_rgb(lab)), 0, 255).astype(np.uint8)


def fetch_image(scene_id, scene, image, cache):
    """the true-colour image a palette is read from, as 8-bit RGB: the
    scene's 1/8 overview (80 m, 1373 pixels square) without its middle row
    and column, so 1372 square; or a 1372 pixel square window of its 1/2
    overview (20 m) whose top left corner is `window` (column, row) in
    full-resolution pixels"""
    name = scene_id + ('' if image['overview'] == 8 else '_z%d_%d_%d' % (image['overview'], *image['window'])) + '.png'
    path = os.path.join(cache, name)
    if os.path.exists(path):
        return np.asarray(Image.open(path).convert('RGB'))
    os.environ.setdefault('GDAL_DISABLE_READDIR_ON_OPEN', 'EMPTY_DIR')
    os.environ.setdefault('GDAL_HTTP_MAX_RETRY', '4')
    os.environ.setdefault('GDAL_HTTP_RETRY_DELAY', '2')
    import rasterio
    from rasterio.windows import Window
    url = '/vsicurl/' + scene['href']
    if image['overview'] == 8:
        with rasterio.open(url, overview_level=2) as ds:
            a = ds.read()
        middle = a.shape[1] // 2
        a = np.delete(np.delete(a, middle, axis=1), middle, axis=2)
    else:
        column, row = image['window']
        with rasterio.open(url, overview_level=0) as ds:
            a = ds.read(window=Window(column // 2, row // 2, 1372, 1372))
    rgb = np.ascontiguousarray(a.transpose(1, 2, 0))
    os.makedirs(cache, exist_ok=True)
    # Written under another name first, so an interrupted run leaves no
    # partial image behind.
    part = path + '.part'
    Image.fromarray(rgb).save(part, format='PNG')
    os.replace(part, path)
    return rgb


def image_key(entry):
    image = entry['image']
    return (entry['scene'], image['overview'], tuple(image.get('window') or ()))


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--out', default=HERE, help='where to write the .clut files (default: beside this script)')
    parser.add_argument('--cache', default=os.path.join(HERE, '.cache'), help='where to keep the downloaded images (default: .cache beside this script)')
    parser.add_argument('--sources', default=os.path.join(HERE, 'palettes.json'), metavar='FILE', help='the palettes to build (default: palettes.json beside this script)')
    args = parser.parse_args()

    sources = json.load(open(args.sources))
    scenes = sources['scenes']
    categories = sources['categories']
    by_image = {}
    for name, entries in categories.items():
        for index, entry in enumerate(entries):
            by_image.setdefault(image_key(entry), []).append((name, index))
    palettes = {name: [None] * len(entries) for name, entries in categories.items()}
    for n, (key, uses) in enumerate(sorted(by_image.items())):
        scene_id, overview, window = key
        image = {'overview': overview, 'window': list(window) or None}
        lab = image_to_lab(fetch_image(scene_id, scenes[scene_id], image, args.cache))
        levels = {}

        def lab_levels(sigma):
            if sigma not in levels:
                levels[sigma] = blurred(lab, sigma)
            return levels[sigma]

        for name, index in uses:
            palettes[name][index] = render(lab_levels, categories[name][index])
        print(f'{n + 1}/{len(by_image)} {scene_id}', file=sys.stderr)
    os.makedirs(args.out, exist_ok=True)
    for name, entries in palettes.items():
        with open(os.path.join(args.out, name + '.clut'), 'wb') as f:
            f.write(struct.pack('<i', len(entries)))
            for palette in entries:
                f.write(palette.tobytes())


if __name__ == '__main__':
    main()
