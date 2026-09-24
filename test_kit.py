#!/usr/bin/env python3
"""Offline unit tests. Run: python -m unittest -v test_kit.py"""
import unittest
from PIL import Image
from make_overlay import crossfade, render_frame, parse_size


class AlphaTests(unittest.TestCase):
    def test_crossfade_does_not_square_alpha(self):
        opaque = Image.new('RGBA', (4, 4), (255, 200, 100, 255))
        mid = crossfade(opaque, opaque, 0.5)
        self.assertEqual(mid.getpixel((0, 0))[3], 255)

    def test_blend_retains_straight_color(self):
        color = Image.new('RGBA', (4, 4), (255, 0, 0, 255))
        empty = Image.new('RGBA', (4, 4), (0, 0, 0, 0))
        mid = crossfade(color, empty, 0.5).getpixel((0, 0))
        self.assertEqual(mid[0], 255)
        self.assertIn(mid[3], (127, 128))

    def test_all_modes_are_periodic(self):
        for mode in ['carousel', 'float', 'spin']:
            assets = [Image.new('RGBA', (32, 32), (255, 120, 10, 150))]
            if mode == 'carousel':
                assets += [Image.new('RGBA', (32, 32), (10, 120, 255, 180))]
            total = 60 * (len(assets) if mode == 'carousel' else 1)
            for offset in (0, 10, 59):
                a = render_frame(offset, assets, (160, 90), mode, 60, 24)
                b = render_frame(total + offset, assets, (160, 90), mode, 60, 24)
                self.assertEqual(a.tobytes(), b.tobytes())
                self.assertEqual(a.getpixel((0, 0))[3], 0)

    def test_carousel_boundary_not_blank(self):
        assets = [Image.new('RGBA', (32, 32), (255, 200, 100, 255)),
                  Image.new('RGBA', (32, 32), (100, 200, 255, 255))]
        for n in (0, 59, 60, 119):
            im = render_frame(n, assets, (160, 90), 'carousel', 60, 24)
            self.assertGreater(im.getchannel('A').getextrema()[1], 250)

    def test_size(self):
        self.assertEqual(parse_size('1920x1080'), (1920, 1080))
        with self.assertRaises(Exception):
            parse_size('1919x1080')


if __name__ == '__main__':
    unittest.main()
