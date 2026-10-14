# Colour library

These are the palettes that SatelliteMap and SuperColor choose from (their
Library and Library Item). Cartography reads them through a SatelliteMap, and
Weathering and TextureBase's New accentuation read the red channel of Rock
palette 117.

| File | Palettes |
| --- | ---: |
| `Blue.clut` | 118 |
| `Color.clut` | 61 |
| `Green.clut` | 317 |
| `New.clut` | 532 |
| `Rock.clut` | 594 |
| `Sand.clut` | 404 |

Each file is a little-endian `i32` palette count followed by the palettes,
each 512 RGB colours of 3 bytes from its start to its end.

## Where the colours come from

Every palette is the colours along a path through a Copernicus Sentinel-2
Level-2A true-colour image (TCI). The palettes use 486 scenes, taken
between 2023-01-12 and 2024-12-31, from the
[Sentinel-2 Cloud-Optimized GeoTIFFs](https://registry.opendata.aws/sentinel-2-l2a-cogs/)
on the AWS Registry of Open Data, found through Element 84's
[Earth Search](https://earth-search.aws.element84.com/v1) (collection
`sentinel-2-l2a`). The tables at the end list each palette's scene and where
its path runs.

**Contains modified Copernicus Sentinel data 2023–2024.**

The [legal notice on the use of Copernicus Sentinel data](https://sentinels.copernicus.eu/documents/247904/690755/Sentinel_Data_Legal_Notice)
lets anyone reproduce, distribute, adapt and modify the data, commercially or
not, as long as the notice above goes with it. Nerthus is not endorsed by the
European Union or ESA.

## How a palette is read

`palettes.json` gives each scene's image file (`href`) and each palette's
scene, image, path and adjustment:

1. The image is either the scene's TCI at 80 m a pixel: its 1/8 overview
   (1373 pixels square) without its middle row and column, so 1372 pixels
   square; or, with `"overview": 2`, a 1372-pixel square window of its 1/2
   overview (20 m a pixel) whose top left corner is `window` (column, row),
   in full-resolution pixels.
2. The image is converted to CIELAB (sRGB, D65). It is blurred with a
   Gaussian whose standard deviation is 0, 1, 2 or 4 pixels: the largest that
   is at most half the mean distance between the palette's samples (the
   length of the line through them divided by 512).
3. The path runs from `path`'s first point (x, y, in pixels of that image,
   0 at the middle of the top left pixel, y pointing down) to its second,
   along a quadratic Bézier curve. Its control point is `bend` chord lengths
   (at most 0.2) from the chord's middle, square to the chord: a positive
   bend curves to the right of the direction of travel. `speeds` spreads the
   curve's parameter over the palette: the parameter moves through each
   quarter of the palette at a constant rate, proportional to the
   exponential of that quarter's value (each from ln ½ to ln 2).
4. The palette's 512 colours are read at the middle of each 1/512 of the
   palette, bilinearly interpolated.
5. They are adjusted in CIELAB: L\* becomes 50 + `contrast` × (L\* − 50) +
   `lightness`, and a\* and b\* become `saturation` × a\* and
   `saturation` × b\*, plus `white_balance`. Contrast is from 0.6 to 1.35,
   lightness from −15 to 15, saturation from 0.6 to 1.6 and each white
   balance term from −6 to 6.
6. They are converted back to sRGB, clipped, and rounded to bytes.

`start` and `end` give where the path starts and ends, as latitude and
longitude. Each scene's `area` is the place it was picked for, which can be
some way from the paths read from it.

## How the paths were chosen

These palettes replace the colour library copied from Gaea 2, which Nerthus
no longer includes. So that projects keep much the same look, each palette
is the path that comes closest to the old palette with the same number,
both in its colours and in how much they change from one sample to the next,
so that striped palettes stay striped and smooth ones smooth. Rock palette
117 was also matched on its red channel, as it is and stretched to the full
range, since that channel is the curve Weathering and TextureBase use.

The search covered 1,284 images of 645 scenes, picked for places with
rock, sand, ice, vegetation, water or coloured ground, mostly in two
seasons, at 80 m and 20 m a pixel. It looked first among straight paths,
then refined the best ones' ends, bend, speeds and adjustment. The old
palettes were only the target of the search and of each palette's
adjustment: every colour is read from the imagery.

Each palette's mean CIEDE2000 difference from the old one:

| Category | Median | 90th percentile |
| --- | ---: | ---: |
| Blue | 3.9 | 7.8 |
| Color | 5.8 | 9.6 |
| Green | 3.3 | 5.6 |
| New | 2.3 | 4.4 |
| Rock | 3.6 | 7.3 |
| Sand | 3.0 | 5.8 |

Projects saved by Gaea keep their palette numbers, which point at these
palettes.

## Rebuilding

`build_palettes.py` rebuilds the `.clut` files from `palettes.json`. It
downloads the 740 images it needs into `.cache` (about 2.3 GB):

```sh
pip install numpy scipy pillow rasterio
python3 build_palettes.py
```

## Palettes

For each palette: its scene (the Earth Search item: satellite, tile, date of
acquisition, sequence and level), the image the path runs through, where the
path starts and ends (latitude, longitude), and the nearest of the places
searched, when one is within 30 km of the path's middle.

### Blue

| # | Scene | Image | Start | End | Near |
| ---: | --- | --- | --- | --- | --- |
| 0 | S2B_44SKC_20240508_0_L2A | 20 m window | 33.6728, 78.7342 | 33.6825, 78.7284 | Pangong Tso, Ladakh (11 km) |
| 1 | S2B_16QCJ_20240324_0_L2A | 80 m | 21.5806, -88.3767 | 21.6477, -88.4459 |  |
| 2 | S2B_36RXR_20240330_0_L2A | 20 m window | 28.0139, 34.5815 | 28.0042, 34.5758 | Strait of Tiran, Red Sea (7 km) |
| 3 | S2A_43TEM_20240621_0_L2A | 80 m | 46.6672, 76.1869 | 46.6823, 76.3130 |  |
| 4 | S2B_18QZJ_20230319_0_L2A | 80 m | 21.2933, -71.9627 | 21.2818, -71.9854 | Caicos Bank, Turks and Caicos (20 km) |
| 5 | S2B_47UNS_20240818_0_L2A | 80 m | 50.7621, 99.6282 | 50.7816, 99.6152 |  |
| 6 | S2A_18FXK_20240516_0_L2A | 20 m window | -50.4865, -72.8782 | -50.4881, -72.8980 | Perito Moreno and Lago Argentino, Patagonia (9 km) |
| 7 | S2B_18QZJ_20230319_0_L2A | 80 m | 21.1764, -71.2621 | 21.1873, -71.2769 |  |
| 8 | S2A_46RBT_20240309_0_L2A | 80 m | 29.1801, 90.5303 | 29.1747, 90.5531 | Yamdrok Tso, Tibet (30 km) |
| 9 | S2B_33XWH_20230705_1_L2A | 20 m window | 78.5670, 15.9419 | 78.5694, 15.9271 | Billefjorden, Svalbard (13 km) |
| 10 | S2A_58KFA_20230408_0_L2A | 80 m | -22.3543, 166.3771 | -22.4139, 166.3410 |  |
| 11 | S2A_38LPQ_20240907_0_L2A | 80 m | -9.4566, 46.4436 | -9.4529, 46.4729 | Aldabra Atoll, Seychelles (12 km) |
| 12 | S2A_16XDP_20240706_1_L2A | 80 m | 80.0924, -88.2861 | 80.0997, -88.1599 |  |
| 13 | S2B_32VLP_20231112_0_L2A | 80 m | 61.7999, 6.8275 | 61.8559, 6.7995 | Jostedalsbreen, Norway (17 km) |
| 14 | S2A_19PGP_20240109_0_L2A | 20 m window | 11.7680, -66.8717 | 11.7589, -66.8554 | Los Roques Archipelago, Venezuela (16 km) |
| 15 | S2B_16QCJ_20240324_0_L2A | 20 m window | 21.6246, -88.2072 | 21.6617, -88.1917 | Ria Lagartos salt ponds, Yucatan (15 km) |
| 16 | S2B_20LNP_20240901_0_L2A | 80 m | -10.2866, -62.3685 | -10.2845, -62.3341 | Rondonia fishbone deforestation, Brazil (27 km) |
| 17 | S2B_5KPB_20230823_0_L2A | 20 m window | -16.4766, -151.7239 | -16.4837, -151.7236 | Bora Bora, French Polynesia (3 km) |
| 18 | S2B_32SPD_20240718_0_L2A | 80 m | 34.4884, 10.7146 | 34.4798, 10.9287 |  |
| 19 | S2B_11RQL_20240327_0_L2A | 80 m | 27.2051, -114.4694 | 27.2254, -114.4791 |  |
| 20 | S2A_5QKB_20230112_0_L2A | 80 m | 19.0516, -155.5486 | 19.0611, -155.5778 |  |
| 21 | S2A_59GMM_20230605_0_L2A | 20 m window | -44.0838, 170.1325 | -44.0708, 170.1407 | Lake Pukaki, New Zealand (3 km) |
| 22 | S2A_18FXL_20240516_0_L2A | 20 m window | -49.4230, -73.4016 | -49.4239, -73.4109 | Southern Patagonian Ice Field, Argentina/Chile (8 km) |
| 23 | S2A_59GMM_20230605_0_L2A | 20 m window | -44.1320, 170.1940 | -44.1487, 170.1180 | Lake Pukaki, New Zealand (10 km) |
| 24 | S2B_26WNE_20240707_0_L2A | 80 m | 71.7282, -24.5687 | 71.7130, -24.5794 |  |
| 25 | S2A_46RBT_20240309_0_L2A | 20 m window | 28.8774, 90.6665 | 28.8825, 90.6632 | Yamdrok Tso, Tibet (9 km) |
| 26 | S2B_16QCJ_20240324_0_L2A | 80 m | 21.6011, -88.6544 | 21.5070, -88.5891 |  |
| 27 | S2A_28RCS_20240414_0_L2A | 80 m | 28.2924, -16.4475 | 28.2410, -16.3800 | Teide caldera, Tenerife Spain (22 km) |
| 28 | S2A_38SLH_20240806_0_L2A | 80 m | 38.6622, 43.1541 | 38.6095, 43.1938 | Lake Van, Turkey (24 km) |
| 29 | S2B_47UNS_20240818_0_L2A | 80 m | 51.3882, 100.3128 | 51.3951, 100.5568 |  |
| 30 | S2A_29VPD_20230420_0_L2A | 80 m | 56.8606, -6.0064 | 56.8501, -6.2064 |  |
| 31 | S2A_46RBT_20240309_0_L2A | 20 m window | 28.9559, 90.7559 | 28.9498, 90.7538 | Yamdrok Tso, Tibet (5 km) |
| 32 | S2A_16QDF_20240205_0_L2A | 20 m window | 18.5497, -87.3747 | 18.5448, -87.3684 | Banco Chinchorro, Mexico (6 km) |
| 33 | S2A_43TEM_20240621_0_L2A | 80 m | 46.4467, 76.1006 | 46.7703, 75.8570 |  |
| 34 | S2A_59GMM_20230605_0_L2A | 20 m window | -44.1073, 170.1367 | -44.0975, 170.1403 | Lake Pukaki, New Zealand (6 km) |
| 35 | S2A_46RBU_20240309_0_L2A | 80 m | 30.5905, 90.3351 | 30.5917, 90.2893 |  |
| 36 | S2A_16TEQ_20240713_0_L2A | 80 m | 44.9967, -86.2199 | 45.0732, -86.1109 | Sleeping Bear Dunes, Lake Michigan (18 km) |
| 37 | S2B_16QCJ_20240324_0_L2A | 80 m | 21.5942, -88.0817 | 21.6701, -88.1446 | Ria Lagartos salt ponds, Yucatan (9 km) |
| 38 | S2B_46RBT_20241030_0_L2A | 20 m window | 28.8807, 90.7961 | 28.9079, 90.7723 | Yamdrok Tso, Tibet (10 km) |
| 39 | S2B_36KYB_20231106_0_L2A | 80 m | -21.5790, 35.9796 | -21.6006, 35.9640 |  |
| 40 | S2B_18RTN_20230226_0_L2A | 80 m | 24.9512, -77.0203 | 24.9377, -76.8967 |  |
| 41 | S2A_16QDF_20240205_0_L2A | 20 m window | 18.5958, -87.2811 | 18.5960, -87.2868 | Banco Chinchorro, Mexico (7 km) |
| 42 | S2A_43TEM_20240621_0_L2A | 20 m window | 46.7087, 75.4617 | 46.7109, 75.4518 | Lake Balkhash, Kazakhstan (13 km) |
| 43 | S2B_18QZJ_20230319_0_L2A | 80 m | 21.6212, -72.0199 | 21.6348, -71.9192 | Caicos Bank, Turks and Caicos (21 km) |
| 44 | S2A_22WEB_20240716_0_L2A | 20 m window | 69.2558, -50.0281 | 69.2597, -50.0250 | Ilulissat Icefjord, Greenland (13 km) |
| 45 | S2B_44SKC_20240508_0_L2A | 20 m window | 33.8183, 78.5871 | 33.8183, 78.5825 | Pangong Tso, Ladakh (10 km) |
| 46 | S2A_28WDS_20240614_0_L2A | 20 m window | 64.0829, -16.2827 | 64.0979, -16.2861 | Jokulsarlon and Breidamerkurjokull, Iceland (5 km) |
| 47 | S2B_32VLP_20231112_0_L2A | 80 m | 62.0880, 6.9768 | 62.1070, 6.9518 |  |
| 48 | S2A_18FXJ_20231208_0_L2A | 20 m window | -51.0615, -73.0424 | -51.0547, -73.0417 | Torres del Paine, Chile (3 km) |
| 49 | S2B_51KVB_20240929_0_L2A | 20 m window | -16.4368, 122.8438 | -16.4997, 122.8149 | Cape Leveque, Dampier Peninsula (13 km) |
| 50 | S2A_43TEM_20240621_0_L2A | 80 m | 46.7891, 76.3388 | 46.6448, 76.2362 |  |
| 51 | S2B_48PXR_20230313_0_L2A | 80 m | 9.2737, 106.5830 | 9.8706, 106.8173 | Mekong River mouths, Vietnam (22 km) |
| 52 | S2A_38LPQ_20240907_0_L2A | 80 m | -9.4913, 46.2626 | -9.5006, 46.2698 | Aldabra Atoll, Seychelles (12 km) |
| 53 | S2B_18GXP_20230619_0_L2A | 20 m window | -46.5416, -72.6680 | -46.5445, -72.6623 | Lago General Carrera, Chile (7 km) |
| 54 | S2A_36RXR_20240911_0_L2A | 20 m window | 27.9551, 34.6565 | 27.9616, 34.6613 | Strait of Tiran, Red Sea (11 km) |
| 55 | S2B_26WNE_20240707_0_L2A | 20 m window | 71.4938, -25.8342 | 71.4853, -25.8220 | Scoresby Sund inner fjords, Greenland (12 km) |
| 56 | S2A_19PGP_20240109_0_L2A | 20 m window | 11.9672, -66.6571 | 11.9648, -66.6528 | Los Roques Archipelago, Venezuela (17 km) |
| 57 | S2B_50KMC_20241015_0_L2A | 20 m window | -20.6774, 116.8328 | -20.6895, 116.7856 | Dampier salt ponds, Pilbara (9 km) |
| 58 | S2B_47UNS_20240818_0_L2A | 20 m window | 51.0250, 100.2829 | 51.0204, 100.2915 | Lake Khovsgol, Mongolia (15 km) |
| 59 | S2A_18GXP_20241225_0_L2A | 20 m window | -46.6014, -72.5944 | -46.5982, -72.6042 | Lago General Carrera, Chile (8 km) |
| 60 | S2B_32TMS_20241231_0_L2A | 80 m | 46.4958, 8.7452 | 46.5037, 8.7101 |  |
| 61 | S2A_18FXJ_20231208_0_L2A | 20 m window | -51.0983, -73.0350 | -51.1016, -73.0387 | Torres del Paine, Chile (6 km) |
| 62 | S2B_36KYB_20231106_0_L2A | 80 m | -21.0849, 35.1070 | -21.0121, 35.1044 |  |
| 63 | S2A_58KFA_20230408_0_L2A | 80 m | -22.6354, 166.5786 | -22.6343, 166.6087 |  |
| 64 | S2A_51LWC_20241021_0_L2A | 80 m | -15.5658, 123.4203 | -15.6780, 124.0032 |  |
| 65 | S2A_18FXK_20240516_0_L2A | 20 m window | -50.4933, -72.9296 | -50.4959, -72.9213 | Perito Moreno and Lago Argentino, Patagonia (7 km) |
| 66 | S2A_22WEB_20240716_0_L2A | 80 m | 69.3699, -50.8669 | 69.3519, -50.9556 |  |
| 67 | S2B_32SPD_20240718_0_L2A | 80 m | 34.6750, 11.1736 | 34.4389, 11.0561 | Kerkennah Islands, Tunisia (18 km) |
| 68 | S2B_39QYG_20240405_0_L2A | 20 m window | 24.3740, 53.1988 | 24.3666, 53.2580 | Marawah coast, Abu Dhabi (15 km) |
| 69 | S2A_51LWC_20241021_0_L2A | 80 m | -15.5529, 124.0053 | -15.5444, 123.8880 |  |
| 70 | S2B_33XXJ_20240802_0_L2A | 80 m | 79.3371, 22.1070 | 79.1277, 22.3420 |  |
| 71 | S2B_5KPB_20230823_0_L2A | 20 m window | -16.5061, -151.7736 | -16.5100, -151.7718 | Bora Bora, French Polynesia (4 km) |
| 72 | S2A_19PGP_20240109_0_L2A | 20 m window | 11.8927, -66.7974 | 11.8835, -66.7839 | Los Roques Archipelago, Venezuela (6 km) |
| 73 | S2A_50SPG_20240425_0_L2A | 80 m | 37.1067, 119.0932 | 37.2109, 119.0812 |  |
| 74 | S2A_51KWB_20240613_0_L2A | 20 m window | -16.5217, 123.4057 | -16.5169, 123.3895 | Buccaneer Archipelago, Kimberley Australia (24 km) |
| 75 | S2A_53HKE_20240827_1_L2A | 20 m window | -31.9781, 132.8773 | -31.9714, 132.8018 | Lake MacDonnell, South Australia (13 km) |
| 76 | S2A_34KFD_20240214_0_L2A | 20 m window | -19.1894, 22.8850 | -19.2375, 22.8917 | Okavango Delta, Botswana (10 km) |
| 77 | S2B_19GBR_20240517_0_L2A | 80 m | -39.8296, -71.9592 | -40.0037, -72.1394 |  |
| 78 | S2A_42WWC_20240715_0_L2A | 80 m | 70.0817, 70.8749 | 70.1053, 70.5705 | Yamal Peninsula tundra, Russia (29 km) |
| 79 | S2A_18FXL_20240516_0_L2A | 80 m | -49.0129, -72.2935 | -49.1501, -72.2651 |  |
| 80 | S2A_22WEV_20240812_0_L2A | 20 m window | 66.9928, -49.8264 | 67.0011, -49.8284 | Kangerlussuaq ice margin, Greenland (14 km) |
| 81 | S2B_28WDT_20231030_0_L2A | 20 m window | 65.5636, -16.9492 | 65.5542, -16.9616 | Myvatn lava and lake, Iceland (5 km) |
| 82 | S2A_12SWJ_20240211_0_L2A | 20 m window | 38.8969, -110.8586 | 38.8777, -110.8721 | San Rafael Swell, Utah USA (11 km) |
| 83 | S2A_59GMM_20230605_0_L2A | 20 m window | -44.0888, 170.1713 | -44.1126, 170.1916 | Lake Pukaki, New Zealand (6 km) |
| 84 | S2B_41XMD_20240731_0_L2A | 80 m | 75.0539, 59.7822 | 75.0902, 59.6611 |  |
| 85 | S2A_30VVJ_20230226_0_L2A | 20 m window | 57.1443, -3.6454 | 57.1380, -3.6405 | Cairngorms heather moorland, Scotland (10 km) |
| 86 | S2B_20LNP_20240901_0_L2A | 20 m window | -10.2529, -62.5731 | -10.2479, -62.5694 | Rondonia fishbone deforestation, Brazil (6 km) |
| 87 | S2A_46WEB_20240913_0_L2A | 80 m | 69.3164, 94.2941 | 69.3098, 94.5251 |  |
| 88 | S2A_59GMM_20230605_0_L2A | 20 m window | -44.1400, 170.1574 | -44.0039, 170.2259 | Lake Pukaki, New Zealand (4 km) |
| 89 | S2A_51LWC_20241021_0_L2A | 80 m | -16.1282, 123.1442 | -16.0262, 123.6811 | Buccaneer Archipelago, Kimberley (23 km) |
| 90 | S2B_36RXR_20240330_0_L2A | 20 m window | 27.9771, 34.6142 | 27.9748, 34.6092 | Strait of Tiran, Red Sea (7 km) |
| 91 | S2B_20TMS_20241019_0_L2A | 20 m window | 46.4687, -63.5382 | 46.4718, -63.5438 | Prince Edward Island (13 km) |
| 92 | S2B_46RBT_20241030_0_L2A | 20 m window | 28.9587, 90.7084 | 28.9571, 90.7025 | Yamdrok Tso, Tibet (1 km) |
| 93 | S2A_19PGP_20240109_0_L2A | 20 m window | 11.8250, -66.7111 | 11.8517, -66.8075 | Los Roques Archipelago, Venezuela (2 km) |
| 94 | S2A_22WEV_20230311_0_L2A | 20 m window | 67.1061, -50.4034 | 67.1079, -50.3901 | Kangerlussuaq (Greenland) (6 km) |
| 95 | S2B_41XMD_20240731_0_L2A | 80 m | 75.0042, 60.9614 | 74.8153, 60.3849 |  |
| 96 | S2A_28RCS_20240414_0_L2A | 80 m | 28.4440, -16.9567 | 28.1603, -16.8296 | Teide caldera, Tenerife Spain (25 km) |
| 97 | S2B_12VVJ_20231217_0_L2A | 80 m | 57.5623, -112.0911 | 57.5417, -112.0806 |  |
| 98 | S2B_45VXJ_20240129_0_L2A | 20 m window | 61.5061, 89.7689 | 61.5166, 89.8230 | Yenisei taiga, Russia (11 km) |
| 99 | S2B_51KVB_20240929_0_L2A | 80 m | -16.5994, 123.0904 | -16.8039, 122.9913 |  |
| 100 | S2A_33XWG_20240718_2_L2A | 20 m window | 78.2405, 17.1430 | 78.2459, 17.0839 | Nordenskiold Land, Svalbard Norway (9 km) |
| 101 | S2B_32TMS_20241231_0_L2A | 80 m | 46.0414, 8.0493 | 46.0611, 8.0752 |  |
| 102 | S2A_16XDP_20240706_1_L2A | 20 m window | 79.5085, -90.2601 | 79.5013, -90.2607 | Axel Heiberg Island, Nunavut Canada (5 km) |
| 103 | S2B_18QZJ_20230319_0_L2A | 20 m window | 21.5186, -71.9538 | 21.4878, -71.9699 | Caicos Bank, Turks and Caicos (9 km) |
| 104 | S2A_7VEG_20240216_0_L2A | 80 m | 59.7519, -140.1116 | 59.7571, -140.0661 | Malaspina Glacier, Alaska (24 km) |
| 105 | S2B_45VXJ_20240129_0_L2A | 80 m | 61.9838, 90.1883 | 61.9990, 90.2246 |  |
| 106 | S2A_16QDF_20240205_0_L2A | 20 m window | 18.5676, -87.3569 | 18.5611, -87.3468 | Banco Chinchorro, Mexico (4 km) |
| 107 | S2A_29VPE_20230420_0_L2A | 20 m window | 57.7666, -6.9011 | 57.7617, -6.8943 | Sound of Harris, Outer Hebrides (9 km) |
| 108 | S2A_19PGP_20230504_0_L2A | 20 m window | 11.9104, -66.7350 | 11.9167, -66.7356 | Los Roques Archipelago, Venezuela (7 km) |
| 109 | S2A_46RBT_20240309_0_L2A | 20 m window | 28.8669, 90.6141 | 28.8525, 90.6336 | Yamdrok Tso, Tibet (12 km) |
| 110 | S2B_17QNG_20240514_0_L2A | 80 m | 23.6893, -80.3091 | 23.6888, -80.3349 | Cay Sal Bank, Bahamas (24 km) |
| 111 | S2B_26WNE_20240707_0_L2A | 20 m window | 71.4629, -25.7817 | 71.4613, -25.7962 | Scoresby Sund inner fjords, Greenland (11 km) |
| 112 | S2B_50KMC_20240208_0_L2A | 80 m | -20.5687, 116.8958 | -20.4570, 116.7093 | Dampier salt ponds, Pilbara (27 km) |
| 113 | S2B_45VXJ_20240129_0_L2A | 20 m window | 61.5059, 89.7938 | 61.5067, 89.7844 | Yenisei taiga, Russia (11 km) |
| 114 | S2B_18QZJ_20230319_0_L2A | 20 m window | 21.3735, -71.8091 | 21.3573, -71.8558 | Caicos Bank, Turks and Caicos (12 km) |
| 115 | S2A_51LWC_20241021_0_L2A | 80 m | -15.9141, 123.7549 | -16.1577, 123.9285 |  |
| 116 | S2A_36RXR_20240911_0_L2A | 20 m window | 27.9628, 34.6575 | 27.9660, 34.6525 | Strait of Tiran, Red Sea (10 km) |
| 117 | S2B_18RTN_20230226_0_L2A | 20 m window | 24.6518, -77.6983 | 24.6466, -77.6984 | Andros and Tongue of the Ocean, Bahamas (16 km) |

### Color

| # | Scene | Image | Start | End | Near |
| ---: | --- | --- | --- | --- | --- |
| 0 | S2B_37PHN_20241130_0_L2A | 20 m window | 11.5828, 42.4997 | 11.5916, 42.5141 | Lake Assal, Djibouti (12 km) |
| 1 | S2B_54HXF_20240422_0_L2A | 80 m | -35.2960, 142.8618 | -35.3449, 142.8868 | Lake Tyrrell, Victoria (4 km) |
| 2 | S2B_13SDT_20240904_0_L2A | 20 m window | 33.7822, -105.8862 | 33.7618, -105.9241 | Carrizozo Malpais lava flow, New Mexico USA (9 km) |
| 3 | S2B_52KDA_20240314_0_L2A | 20 m window | -22.5733, 128.5999 | -22.5776, 128.5928 | Lake Mackay (Australia) (8 km) |
| 4 | S2B_51KVQ_20240110_0_L2A | 20 m window | -23.4126, 122.9122 | -23.4259, 122.9337 | Lake Disappointment (12 km) |
| 5 | S2A_36MZC_20230128_0_L2A | 20 m window | -2.3539, 36.1006 | -2.3756, 36.0558 | Lake Natron, Tanzania (5 km) |
| 6 | S2B_51KVB_20240929_0_L2A | 20 m window | -16.4466, 123.0053 | -16.4531, 123.0075 | Cape Leveque, Dampier Peninsula (10 km) |
| 7 | S2B_39TUL_20241123_0_L2A | 20 m window | 45.9227, 48.8301 | 45.9161, 48.8274 | Volga Delta, Russia (13 km) |
| 8 | S2B_19JEG_20240315_0_L2A | 20 m window | -29.9316, -68.0139 | -29.9322, -68.0306 | Ischigualasto and Talampaya (Argentina) (14 km) |
| 9 | S2B_28WDT_20231030_0_L2A | 20 m window | 65.5303, -17.0449 | 65.5333, -16.9840 | Myvatn lava and lake, Iceland (8 km) |
| 10 | S2A_18FXK_20240516_0_L2A | 20 m window | -50.4338, -73.1596 | -50.4380, -73.1542 | Perito Moreno Glacier and Lago Argentino, Argentina (8 km) |
| 11 | S2B_32VLP_20231112_0_L2A | 80 m | 61.2856, 5.4968 | 61.2572, 5.4396 |  |
| 12 | S2B_51KVQ_20240110_0_L2A | 20 m window | -23.3606, 122.8711 | -23.3739, 122.8778 | Lake Disappointment (15 km) |
| 13 | S2A_47UPS_20240402_0_L2A | 20 m window | 50.9041, 100.6680 | 50.9028, 100.6590 | Lake Khovsgol, Mongolia (16 km) |
| 14 | S2B_8WMB_20241004_0_L2A | 20 m window | 69.1202, -135.3715 | 69.1126, -135.3685 | Mackenzie River Delta, Canada (20 km) |
| 15 | S2B_51KVB_20240929_0_L2A | 80 m | -16.8944, 122.0767 | -17.1448, 122.6816 |  |
| 16 | S2B_28RFT_20240715_0_L2A | 20 m window | 29.0338, -13.6882 | 29.0254, -13.6891 | Timanfaya, Lanzarote Spain (7 km) |
| 17 | S2B_12SVE_20241003_0_L2A | 80 m | 35.4344, -111.3585 | 35.4881, -111.2594 |  |
| 18 | S2B_49MHM_20231027_0_L2A | 80 m | -7.6824, 114.4942 | -7.3681, 114.6706 |  |
| 19 | S2A_18FXK_20240516_0_L2A | 20 m window | -50.5183, -72.9814 | -50.5222, -72.9777 | Perito Moreno glacier (Argentina) (7 km) |
| 20 | S2B_8WMB_20241004_0_L2A | 20 m window | 68.9266, -135.0032 | 68.9239, -135.0196 | Mackenzie River Delta, Canada (8 km) |
| 21 | S2A_30VVJ_20230226_0_L2A | 80 m | 56.8894, -3.9787 | 56.8588, -3.9126 | Cairngorms heather moorland, Scotland (27 km) |
| 22 | S2B_12VVJ_20231217_0_L2A | 20 m window | 56.9757, -111.7344 | 56.9828, -111.7382 | Athabasca oil sands (Canada) (9 km) |
| 23 | S2B_32TMS_20241231_0_L2A | 80 m | 46.2005, 8.3782 | 46.1776, 8.3431 |  |
| 24 | S2B_32VLP_20231112_0_L2A | 80 m | 61.6200, 5.9334 | 61.6043, 5.9182 |  |
| 25 | S2B_8WMB_20241004_0_L2A | 80 m | 68.6956, -134.8339 | 68.7137, -134.8113 | Mackenzie Delta, Canada (24 km) |
| 26 | S2A_13SCS_20240909_0_L2A | 80 m | 33.1827, -106.9786 | 33.2066, -106.9702 |  |
| 27 | S2B_32VLP_20231112_0_L2A | 80 m | 61.5031, 5.5051 | 61.5058, 5.4808 |  |
| 28 | S2B_8WMB_20241004_0_L2A | 20 m window | 69.0321, -135.0358 | 69.0290, -135.0494 | Mackenzie River Delta, Canada (4 km) |
| 29 | S2B_28RFT_20240715_0_L2A | 20 m window | 28.9542, -13.6866 | 28.9492, -13.6888 | Timanfaya, Lanzarote Spain (8 km) |
| 30 | S2B_12STA_20240615_0_L2A | 20 m window | 31.7954, -113.3898 | 31.8043, -113.4039 | Pinacate volcanic field, Sonora Mexico (10 km) |
| 31 | S2A_18FXK_20240516_0_L2A | 20 m window | -50.5809, -72.8728 | -50.5795, -72.8670 | Perito Moreno glacier (Argentina) (17 km) |
| 32 | S2B_28WDT_20231030_0_L2A | 80 m | 65.1596, -15.3692 | 65.1369, -15.3942 |  |
| 33 | S2B_22MEU_20240817_0_L2A | 20 m window | -6.0748, -50.1384 | -6.0875, -50.1259 | Carajas iron mine, Brazil (4 km) |
| 34 | S2A_49KGP_20240107_0_L2A | 80 m | -24.2192, 113.1672 | -23.7155, 113.7478 | Lake MacLeod, Carnarvon (20 km) |
| 35 | S2B_22MEU_20240817_0_L2A | 80 m | -6.0594, -50.1779 | -6.0648, -50.1518 | Carajas iron mine, Brazil (2 km) |
| 36 | S2A_12SWF_20241005_0_L2A | 20 m window | 36.9026, -110.0670 | 36.8477, -110.1022 | Monument Valley (USA) (12 km) |
| 37 | S2A_18WXD_20240818_0_L2A | 20 m window | 70.7254, -71.2389 | 70.7213, -71.3765 | Sam Ford Fjord, Baffin Island (11 km) |
| 38 | S2A_32QMF_20241008_0_L2A | 20 m window | 18.3960, 8.5199 | 18.4205, 8.5438 | Air Mountains, Niger (12 km) |
| 39 | S2B_38LPH_20240919_0_L2A | 20 m window | -15.9113, 46.1647 | -15.9470, 46.1790 | Betsiboka Delta, Madagascar (12 km) |
| 40 | S2B_12SUG_20241003_0_L2A | 20 m window | 37.3551, -113.0979 | 37.3643, -113.0870 | Zion Canyon, Utah USA (15 km) |
| 41 | S2A_19KHQ_20240320_0_L2A | 80 m | -23.2020, -65.0958 | -23.0955, -65.0866 | Quebrada de Humahuaca, Jujuy Argentina (27 km) |
| 42 | S2B_23LMG_20240920_0_L2A | 20 m window | -12.5081, -45.5230 | -12.4774, -45.5433 | Western Bahia centre pivots, Brazil (13 km) |
| 43 | S2B_52KDA_20240314_0_L2A | 80 m | -22.4198, 128.7695 | -22.4366, 128.8052 | Lake Mackay, Australia (12 km) |
| 44 | S2B_22MEU_20240817_0_L2A | 20 m window | -6.0711, -50.1634 | -6.0751, -50.1469 | Carajas iron mine, Brazil (3 km) |
| 45 | S2B_57VXC_20241015_0_L2A | 80 m | 56.6381, 161.6602 | 56.6761, 161.6134 |  |
| 46 | S2A_30VUH_20230420_0_L2A | 80 m | 55.9730, -5.8012 | 56.1830, -5.7521 |  |
| 47 | S2B_28WDT_20231030_0_L2A | 20 m window | 65.5905, -17.1580 | 65.5743, -17.1464 | Myvatn lava and lake, Iceland (7 km) |
| 48 | S2B_26WPE_20240717_0_L2A | 80 m | 71.1090, -23.1645 | 71.1606, -23.1946 | Jameson Land, East Greenland (22 km) |
| 49 | S2B_8WMB_20241004_0_L2A | 20 m window | 68.8994, -135.1313 | 68.9116, -135.1430 | Mackenzie River Delta, Canada (12 km) |
| 50 | S2B_8WMB_20241004_0_L2A | 20 m window | 69.0924, -134.9955 | 69.0765, -134.9819 | Mackenzie River Delta, Canada (9 km) |
| 51 | S2A_48UXD_20230512_0_L2A | 80 m | 53.0337, 106.8364 | 53.0254, 106.8930 |  |
| 52 | S2B_8WMB_20241004_0_L2A | 20 m window | 69.0666, -134.8829 | 69.0484, -135.0748 | Mackenzie River Delta, Canada (6 km) |
| 53 | S2B_39TUL_20241123_0_L2A | 20 m window | 45.8721, 48.7599 | 45.8787, 48.7594 | Volga Delta, Russia (5 km) |
| 54 | S2A_18FXK_20240516_0_L2A | 20 m window | -50.5338, -72.8729 | -50.5373, -72.8682 | Perito Moreno and Lago Argentino, Patagonia (13 km) |
| 55 | S2B_57VXC_20241015_0_L2A | 20 m window | 56.0139, 160.9531 | 56.0260, 160.9744 | Klyuchevskaya volcano group, Kamchatka (20 km) |
| 56 | S2B_33XWH_20230705_1_L2A | 80 m | 79.2051, 15.3811 | 79.2209, 15.2950 |  |
| 57 | S2B_12VVJ_20231217_0_L2A | 20 m window | 56.9435, -111.5400 | 56.9464, -111.5555 | Athabasca oil sands mines, Alberta Canada (6 km) |
| 58 | S2B_28RFT_20240715_0_L2A | 80 m | 28.8983, -13.8667 | 28.8835, -13.8841 | Timanfaya, Lanzarote Spain (17 km) |
| 59 | S2B_28WDT_20231030_0_L2A | 20 m window | 65.5483, -17.0712 | 65.5478, -17.0896 | Myvatn lava and lake, Iceland (7 km) |
| 60 | S2B_33KVS_20241114_0_L2A | 20 m window | -21.2205, 14.5157 | -21.2151, 14.5199 | Brandberg massif, Namibia (8 km) |

### Green

| # | Scene | Image | Start | End | Near |
| ---: | --- | --- | --- | --- | --- |
| 0 | S2A_30VVJ_20230616_0_L2A | 80 m | 57.1589, -4.0388 | 57.2308, -4.0027 | Cairngorms heather moorland, Scotland (28 km) |
| 1 | S2A_32TQS_20230910_1_L2A | 20 m window | 46.4127, 11.9793 | 46.4229, 11.9689 | Dolomites, Italy (13 km) |
| 2 | S2A_55GDN_20240309_0_L2A | 80 m | -42.9256, 146.4499 | -42.5240, 146.3783 |  |
| 3 | S2B_27VXL_20240629_0_L2A | 20 m window | 63.7635, -17.4265 | 63.7613, -17.4340 | Skeidararsandur, Iceland (17 km) |
| 4 | S2B_39TUL_20241123_0_L2A | 80 m | 45.7002, 49.0776 | 45.7547, 48.9893 | Volga Delta, Russia (29 km) |
| 5 | S2A_34KFD_20240214_0_L2A | 20 m window | -19.3456, 22.9487 | -19.4152, 22.8955 | Okavango Delta, Botswana (9 km) |
| 6 | S2B_36MXC_20240311_0_L2A | 20 m window | -2.4303, 34.6429 | -2.4381, 34.6562 | Serengeti plains, Tanzania (18 km) |
| 7 | S2B_60HUB_20240509_0_L2A | 80 m | -39.4126, 175.8923 | -39.6115, 175.8325 |  |
| 8 | S2A_55GDN_20240309_0_L2A | 20 m window | -42.9449, 146.0490 | -42.9085, 145.9767 | Southwest Tasmania wilderness, Australia (8 km) |
| 9 | S2A_5QKC_20241003_0_L2A | 80 m | 19.7995, -155.2806 | 19.9441, -155.3843 | Mauna Kea summit, Hawaii USA (15 km) |
| 10 | S2A_49RDH_20240312_0_L2A | 20 m window | 24.8482, 110.5017 | 24.8377, 110.4817 | Yangshuo-Guilin karst, China (8 km) |
| 11 | S2B_32UME_20240501_0_L2A | 20 m window | 53.7683, 7.7889 | 53.7750, 7.7776 | Wadden Sea mudflats, Germany (18 km) |
| 12 | S2A_33SWB_20241015_0_L2A | 20 m window | 37.7664, 15.0430 | 37.7490, 15.0572 | Mount Etna, Sicily Italy (4 km) |
| 13 | S2A_48QYJ_20240305_0_L2A | 20 m window | 20.8949, 107.2259 | 20.9362, 107.1744 | Ha Long Bay, Vietnam (13 km) |
| 14 | S2B_32MNC_20240818_0_L2A | 20 m window | -2.4084, 9.6226 | -2.4116, 9.6093 | Loango coast, Gabon (15 km) |
| 15 | S2A_60HUB_20230907_0_L2A | 20 m window | -39.2459, 175.7987 | -39.2566, 175.7329 | Tongariro volcanic plateau, New Zealand (15 km) |
| 16 | S2A_16QBF_20240329_0_L2A | 80 m | 18.7773, -89.0343 | 18.3457, -89.1972 |  |
| 17 | S2B_58KFA_20241004_0_L2A | 80 m | -22.2871, 166.1064 | -22.4045, 166.0200 |  |
| 18 | S2A_27WWM_20240707_0_L2A | 80 m | 64.0463, -20.5031 | 64.1012, -20.5236 |  |
| 19 | S2B_32WPA_20240609_0_L2A | 80 m | 68.2264, 13.7401 | 68.2515, 13.7195 | Lofoten Islands, Norway (12 km) |
| 20 | S2A_16RBT_20240511_0_L2A | 80 m | 29.7953, -89.0100 | 29.6885, -89.2564 |  |
| 21 | S2A_5QKC_20241003_0_L2A | 20 m window | 19.9585, -155.5711 | 19.9612, -155.5521 | Mauna Kea summit, Hawaii USA (18 km) |
| 22 | S2A_39TUL_20240701_0_L2A | 20 m window | 45.9316, 48.6399 | 45.9284, 48.6238 | Volga Delta, Russia (10 km) |
| 23 | S2B_30PUB_20241107_0_L2A | 80 m | 15.1044, -4.4055 | 15.0440, -4.3606 | Inner Niger Delta near Lake Debo, Mali (24 km) |
| 24 | S2A_33XWG_20240718_2_L2A | 80 m | 77.9532, 18.7428 | 78.1769, 19.2876 |  |
| 25 | S2A_49KGR_20240206_0_L2A | 20 m window | -22.4968, 113.6730 | -22.5045, 113.6901 | Ningaloo Reef, Australia (12 km) |
| 26 | S2A_21KVA_20240530_0_L2A | 20 m window | -17.8242, -57.1350 | -17.8930, -57.3118 | Paraguay River Pantanal wetlands, Brazil (7 km) |
| 27 | S2A_49MGM_20230624_0_L2A | 80 m | -7.8851, 113.4190 | -7.7090, 113.2680 |  |
| 28 | S2A_49RDH_20240312_0_L2A | 80 m | 24.8153, 110.7865 | 25.0627, 110.4131 | Yangshuo-Guilin karst, China (16 km) |
| 29 | S2A_20KKU_20241115_0_L2A | 80 m | -24.0985, -65.5786 | -24.2466, -65.3445 |  |
| 30 | S2A_53HLE_20240110_0_L2A | 80 m | -32.3467, 133.9571 | -32.4896, 133.2328 |  |
| 31 | S2B_26WPE_20240717_0_L2A | 20 m window | 71.2670, -23.6284 | 71.3436, -23.7414 | Jameson Land, East Greenland (7 km) |
| 32 | S2A_39TUL_20240701_0_L2A | 80 m | 45.9473, 49.4846 | 45.8473, 49.3881 |  |
| 33 | S2A_34KFD_20240214_0_L2A | 80 m | -19.4969, 22.0795 | -19.3937, 22.0807 |  |
| 34 | S2A_5QKC_20241003_0_L2A | 20 m window | 19.9196, -155.3752 | 19.9828, -155.3740 | Mauna Kea summit, Hawaii USA (18 km) |
| 35 | S2A_34KFD_20240214_0_L2A | 20 m window | -19.2761, 22.9089 | -19.2787, 22.9183 | Okavango Delta, Botswana (3 km) |
| 36 | S2B_44TLP_20240421_0_L2A | 80 m | 43.6523, 78.9214 | 43.5343, 78.8534 |  |
| 37 | S2A_37TGJ_20240819_0_L2A | 20 m window | 43.3865, 42.5336 | 43.4314, 42.5251 | Mount Elbrus, Caucasus Russia (9 km) |
| 38 | S2B_17RNJ_20231017_0_L2A | 20 m window | 25.5305, -80.8632 | 25.5785, -80.8230 | Everglades sawgrass, Florida, USA (5 km) |
| 39 | S2A_11RQL_20240819_0_L2A | 80 m | 27.6049, -114.1274 | 27.6051, -114.1974 | Laguna Ojo de Liebre and Guerrero Negro saltworks, Baja California (24 km) |
| 40 | S2A_21MXT_20241129_0_L2A | 20 m window | -1.9461, -55.4690 | -1.9525, -55.4486 | Amazon River near Obidos, Brazil (9 km) |
| 41 | S2A_21MXT_20241129_0_L2A | 20 m window | -2.0091, -55.4297 | -2.0008, -55.4464 | Amazon River near Obidos, Brazil (15 km) |
| 42 | S2B_45QYE_20241116_0_L2A | 80 m | 22.5012, 89.0790 | 22.3474, 89.0116 |  |
| 43 | S2A_35KLT_20240122_0_L2A | 80 m | -20.0768, 25.7843 | -20.1758, 25.7691 |  |
| 44 | S2A_16RBT_20240511_0_L2A | 20 m window | 29.1710, -89.3511 | 29.1671, -89.3591 | Mississippi bird's-foot delta, USA (10 km) |
| 45 | S2A_4NFH_20230820_0_L2A | 80 m | 1.8050, -157.3217 | 1.8023, -157.3445 | Kiritimati Atoll (10 km) |
| 46 | S2A_21MXT_20241129_0_L2A | 20 m window | -2.0153, -55.4247 | -1.9928, -55.4357 | Amazon River near Obidos, Brazil (15 km) |
| 47 | S2A_33XWG_20240718_2_L2A | 20 m window | 78.3173, 16.2041 | 78.3501, 16.3367 | Nordenskiold Land, Svalbard Norway (12 km) |
| 48 | S2A_13RBL_20240929_0_L2A | 80 m | 27.6329, -107.3193 | 27.5015, -107.5823 |  |
| 49 | S2A_32VMP_20240919_0_L2A | 20 m window | 61.6966, 8.3539 | 61.6867, 8.3570 | Jotunheimen, Norway (10 km) |
| 50 | S2A_21MXT_20241129_0_L2A | 20 m window | -2.0362, -55.4141 | -2.0240, -55.4320 | Amazon River near Obidos, Brazil (18 km) |
| 51 | S2B_42RXR_20241001_0_L2A | 20 m window | 27.2794, 70.4045 | 27.2653, 70.3931 | Thar Desert (India) (13 km) |
| 52 | S2B_47UNS_20240818_0_L2A | 80 m | 50.6388, 99.8863 | 50.6756, 99.9009 |  |
| 53 | S2A_36MYB_20230210_0_L2A | 80 m | -2.8169, 34.9180 | -2.7603, 35.0637 |  |
| 54 | S2A_5QKB_20230112_0_L2A | 20 m window | 19.3642, -155.2162 | 19.3318, -155.2013 | Kilauea caldera and East Rift, Hawaii USA (7 km) |
| 55 | S2A_38KME_20240907_0_L2A | 20 m window | -18.8492, 44.7626 | -18.7981, 44.7752 | Tsingy de Bemaraha, Madagascar (8 km) |
| 56 | S2B_27PZS_20240506_0_L2A | 80 m | 14.4468, -17.5079 | 14.4460, -17.2636 |  |
| 57 | S2A_37PFT_20240522_0_L2A | 20 m window | 15.8727, 40.2714 | 15.8705, 40.2635 | Dahlak Archipelago, Eritrea (15 km) |
| 58 | S2A_30SWG_20240202_0_L2A | 80 m | 37.3793, -2.2784 | 37.4469, -2.2602 |  |
| 59 | S2A_39TUL_20240701_0_L2A | 20 m window | 45.8752, 48.7783 | 45.8897, 48.7626 | Volga Delta, Russia (7 km) |
| 60 | S2A_59GMM_20240220_0_L2A | 20 m window | -43.6524, 170.2663 | -43.6516, 170.2600 | Aoraki / Mount Cook, New Zealand (11 km) |
| 61 | S2B_36RTV_20240724_0_L2A | 20 m window | 30.8485, 30.5954 | 30.8523, 30.5899 | Western Nile Delta edge, Egypt (4 km) |
| 62 | S2A_39TUL_20240701_0_L2A | 80 m | 45.8141, 49.4479 | 45.7833, 49.3293 |  |
| 63 | S2B_5VLE_20240615_0_L2A | 20 m window | 58.4294, -155.1658 | 58.4379, -155.1825 | Valley of Ten Thousand Smokes, Alaska USA (12 km) |
| 64 | S2A_33SWB_20241015_0_L2A | 20 m window | 37.8433, 15.1657 | 37.8677, 15.1160 | Mount Etna, Sicily Italy (17 km) |
| 65 | S2A_39TUL_20240701_0_L2A | 80 m | 45.5853, 49.5142 | 45.3358, 49.2313 |  |
| 66 | S2A_36MYB_20230210_0_L2A | 80 m | -3.0878, 34.8412 | -3.1397, 34.8015 |  |
| 67 | S2A_15TUG_20240523_0_L2A | 80 m | 41.9340, -95.3904 | 41.9363, -95.3685 |  |
| 68 | S2B_38UPU_20240520_0_L2A | 80 m | 47.7798, 46.6197 | 47.8825, 46.5761 |  |
| 69 | S2A_32KRE_20240824_0_L2A | 80 m | -18.7845, 12.2772 | -18.4678, 12.1990 |  |
| 70 | S2B_45QYE_20241116_0_L2A | 80 m | 22.3510, 89.0423 | 22.4245, 89.1848 |  |
| 71 | S2A_48UXE_20240913_0_L2A | 80 m | 53.5978, 107.5226 | 53.6164, 107.3890 |  |
| 72 | S2B_13TGG_20240606_0_L2A | 20 m window | 42.1190, -101.3160 | 42.1151, -101.3334 | Nebraska Sand Hills, USA (6 km) |
| 73 | S2B_11UMT_20240707_0_L2A | 80 m | 52.0683, -117.6648 | 52.0551, -117.6408 | Columbia Icefield, Alberta/BC Canada (26 km) |
| 74 | S2A_37TGJ_20240819_0_L2A | 20 m window | 43.4221, 42.3269 | 43.4245, 42.3315 | Mount Elbrus, Caucasus Russia (13 km) |
| 75 | S2A_39TYF_20240824_0_L2A | 80 m | 41.2466, 54.2931 | 41.2291, 54.3602 |  |
| 76 | S2B_45QYE_20241116_0_L2A | 20 m window | 21.8917, 89.2368 | 21.9205, 89.2525 | Sundarbans mangroves, Bangladesh and India (5 km) |
| 77 | S2B_22KGU_20240114_0_L2A | 20 m window | -24.2465, -48.2070 | -24.2534, -48.2095 | Serra do Mar Atlantic Forest, Brazil (11 km) |
| 78 | S2B_17RNJ_20230301_0_L2A | 20 m window | 25.6382, -80.8067 | 25.6538, -80.7938 | Everglades sawgrass, Florida, USA (7 km) |
| 79 | S2B_13TGG_20240606_0_L2A | 20 m window | 41.9938, -101.5811 | 41.9833, -101.5833 | Nebraska Sand Hills, USA (19 km) |
| 80 | S2B_38SNG_20240706_0_L2A | 80 m | 37.7056, 45.4111 | 37.3560, 45.4077 | Lake Urmia, Iran (14 km) |
| 81 | S2A_21MXT_20241129_0_L2A | 20 m window | -1.9564, -55.5942 | -1.9516, -55.5880 | Amazon River near Obidos, Brazil (10 km) |
| 82 | S2B_13TGG_20240606_0_L2A | 20 m window | 42.1276, -101.5335 | 42.1354, -101.5356 | Nebraska Sand Hills, USA (12 km) |
| 83 | S2B_39TUL_20241123_0_L2A | 80 m | 45.1861, 49.4021 | 45.1756, 49.6608 |  |
| 84 | S2A_46SFF_20240425_0_L2A | 20 m window | 36.8537, 95.1123 | 36.8430, 95.1197 | Qarhan Salt Lake (China) (6 km) |
| 85 | S2A_11TMN_20240612_0_L2A | 80 m | 47.4241, -118.2582 | 47.4194, -118.2307 |  |
| 86 | S2A_5QKB_20230112_0_L2A | 20 m window | 19.4465, -155.0660 | 19.4371, -155.0955 | Kilauea lava flows, Hawaii (5 km) |
| 87 | S2B_42RXR_20241001_0_L2A | 20 m window | 27.1185, 70.1966 | 27.1504, 70.2229 | Thar Desert (India) (12 km) |
| 88 | S2B_33PVQ_20240527_0_L2A | 20 m window | 13.1438, 14.2556 | 13.1539, 14.2544 | Lake Chad (Chad/Nigeria) (7 km) |
| 89 | S2A_36MZB_20230128_0_L2A | 20 m window | -2.9321, 35.9645 | -2.9238, 35.9601 | Ol Doinyo Lengai, Tanzania (20 km) |
| 90 | S2A_48QYJ_20240305_0_L2A | 80 m | 20.7398, 106.9609 | 20.7667, 107.1398 | Ha Long Bay, Vietnam (12 km) |
| 91 | S2B_21HVB_20240916_0_L2A | 80 m | -34.9026, -57.9344 | -34.9305, -57.9566 |  |
| 92 | S2B_23MQT_20230628_0_L2A | 80 m | -2.7384, -42.3469 | -2.7288, -42.3873 |  |
| 93 | S2A_31TFJ_20241229_0_L2A | 80 m | 43.7066, 5.2523 | 43.7761, 5.2389 |  |
| 94 | S2B_41XMD_20240731_0_L2A | 20 m window | 75.4293, 60.7514 | 75.4344, 60.7340 | Novaya Zemlya ice cap (10 km) |
| 95 | S2B_49KGP_20240809_0_L2A | 20 m window | -24.0863, 113.5983 | -24.1076, 113.6046 | Lake MacLeod, Carnarvon (12 km) |
| 96 | S2A_47SNA_20240330_0_L2A | 80 m | 36.9334, 99.7693 | 36.9705, 99.7802 |  |
| 97 | S2A_5QKB_20230112_0_L2A | 80 m | 19.8328, -155.6318 | 19.7546, -155.6603 | Mauna Kea summit, Hawaii USA (19 km) |
| 98 | S2A_31TFJ_20241229_0_L2A | 20 m window | 43.3799, 4.5728 | 43.3855, 4.5879 | Camargue salt pans, France (8 km) |
| 99 | S2A_11SQR_20240620_0_L2A | 20 m window | 31.6567, -114.6105 | 31.6800, -114.6202 | Colorado River Delta, Gulf of California (13 km) |
| 100 | S2B_45QYE_20240430_0_L2A | 20 m window | 21.8681, 89.2069 | 21.8629, 89.1925 | Sundarbans mangroves, Bangladesh and India (4 km) |
| 101 | S2A_17RNH_20231101_0_L2A | 20 m window | 24.9259, -80.7469 | 24.9376, -80.7500 | Florida Bay and Upper Keys (6 km) |
| 102 | S2A_51LWC_20241021_0_L2A | 80 m | -16.0101, 123.6099 | -16.0803, 123.5977 |  |
| 103 | S2B_54TXP_20240903_0_L2A | 20 m window | 43.6102, 142.9143 | 43.6042, 142.9107 | Daisetsuzan, Hokkaido, Japan (7 km) |
| 104 | S2A_37TGJ_20240819_0_L2A | 20 m window | 43.4214, 42.3114 | 43.4293, 42.3193 | Mount Elbrus, Caucasus Russia (14 km) |
| 105 | S2A_18LVJ_20240404_0_L2A | 80 m | -14.7393, -75.8156 | -14.7251, -75.8092 |  |
| 106 | S2B_12UVB_20240721_0_L2A | 20 m window | 50.7982, -111.3549 | 50.7900, -111.3483 | Dinosaur Provincial Park badlands, Alberta Canada (12 km) |
| 107 | S2A_12TUK_20240613_0_L2A | 20 m window | 40.6278, -112.1070 | 40.6233, -112.1213 | Bingham Canyon Mine, Utah (12 km) |
| 108 | S2B_45QYE_20240430_0_L2A | 80 m | 21.6880, 89.5247 | 21.7974, 89.6101 |  |
| 109 | S2B_36MZD_20240805_0_L2A | 80 m | -1.0782, 36.3188 | -1.0776, 36.2719 |  |
| 110 | S2A_4NFH_20230820_0_L2A | 20 m window | 1.8554, -157.4843 | 1.8579, -157.5090 | Kiritimati Atoll (11 km) |
| 111 | S2B_40TEQ_20240418_0_L2A | 20 m window | 44.9199, 58.3503 | 44.9192, 58.3869 | Aral Sea west basin, Aralkum (9 km) |
| 112 | S2A_36MYB_20230720_0_L2A | 80 m | -2.9800, 35.6251 | -2.8438, 35.5693 | Ngorongoro Crater, Tanzania (30 km) |
| 113 | S2A_43TFH_20231012_0_L2A | 80 m | 42.4640, 76.2416 | 42.4522, 76.2239 |  |
| 114 | S2B_50PQT_20230805_0_L2A | 80 m | 10.8568, 119.4415 | 10.9025, 119.5091 |  |
| 115 | S2A_32VMP_20240522_0_L2A | 20 m window | 61.6569, 8.1875 | 61.6619, 8.2187 | Jotunheimen, Norway (12 km) |
| 116 | S2A_21MXT_20241129_0_L2A | 20 m window | -2.0188, -55.4682 | -2.0141, -55.4730 | Amazon River near Obidos, Brazil (14 km) |
| 117 | S2B_52LDJ_20240903_0_L2A | 20 m window | -14.9478, 128.2714 | -14.9050, 128.2926 | Cambridge Gulf (Australia) (9 km) |
| 118 | S2A_32TQS_20230910_1_L2A | 80 m | 46.3490, 12.7068 | 46.3633, 12.7458 |  |
| 119 | S2A_46WEB_20240913_0_L2A | 20 m window | 69.0114, 94.2610 | 69.0111, 94.2813 | Putorana Plateau, Siberia Russia (11 km) |
| 120 | S2B_31UFV_20240921_0_L2A | 80 m | 53.4667, 5.8724 | 53.4613, 5.7852 |  |
| 121 | S2B_32UME_20240501_0_L2A | 20 m window | 53.5933, 7.8011 | 53.5898, 7.7941 | Wadden Sea mudflats, Germany (15 km) |
| 122 | S2B_27VXL_20240629_0_L2A | 20 m window | 63.7638, -17.0276 | 63.7358, -17.1594 | Skeidararsandur outwash plain, Iceland (18 km) |
| 123 | S2A_36NZK_20240704_0_L2A | 20 m window | 3.5297, 36.1600 | 3.5722, 36.1629 | Lake Turkana, Kenya (7 km) |
| 124 | S2A_50SPG_20240425_0_L2A | 80 m | 37.4160, 118.8441 | 37.3850, 118.8334 |  |
| 125 | S2A_46SFF_20240425_0_L2A | 20 m window | 36.9164, 95.0939 | 36.9200, 95.0812 | Qarhan salt lake ponds, Qinghai (10 km) |
| 126 | S2A_36MYB_20230720_0_L2A | 20 m window | -3.2196, 35.6210 | -3.2157, 35.6384 | Ngorongoro Crater, Tanzania (7 km) |
| 127 | S2B_27VXL_20240629_0_L2A | 20 m window | 63.8909, -17.1216 | 63.8998, -17.1399 | Skeidararsandur outwash plain, Iceland (3 km) |
| 128 | S2B_44TLP_20240421_0_L2A | 20 m window | 43.3137, 79.0440 | 43.3090, 79.0405 | Charyn Canyon, Kazakhstan (5 km) |
| 129 | S2A_14WPS_20240727_0_L2A | 20 m window | 64.4378, -95.8623 | 64.4564, -95.8867 | Baker Lake tundra, Nunavut Canada (8 km) |
| 130 | S2A_17RNH_20231101_0_L2A | 20 m window | 25.0638, -80.6549 | 25.0652, -80.6488 | Florida Bay, USA (12 km) |
| 131 | S2B_36MZB_20240914_0_L2A | 80 m | -3.1714, 36.5621 | -3.2561, 36.4623 |  |
| 132 | S2A_44SKA_20240513_0_L2A | 80 m | 31.5285, 78.3016 | 31.5660, 78.3006 |  |
| 133 | S2B_42QWM_20241001_0_L2A | 80 m | 24.2105, 69.3088 | 24.2383, 69.3456 |  |
| 134 | S2B_47UNS_20240818_0_L2A | 80 m | 51.3414, 99.4842 | 51.2589, 99.2434 |  |
| 135 | S2B_38SNG_20240706_0_L2A | 20 m window | 37.3147, 45.5759 | 37.3079, 45.5875 | Lake Urmia, Iran (10 km) |
| 136 | S2A_39TUL_20240701_0_L2A | 80 m | 45.1255, 49.6017 | 45.3546, 48.8723 |  |
| 137 | S2A_17RNH_20231101_0_L2A | 80 m | 25.2358, -80.7121 | 25.2578, -80.7017 | Florida Bay, USA (28 km) |
| 138 | S2A_39TUL_20240701_0_L2A | 20 m window | 45.8617, 48.7463 | 45.8910, 48.7083 | Volga Delta, Russia (4 km) |
| 139 | S2B_42QWM_20241001_0_L2A | 80 m | 23.9983, 69.9001 | 24.0427, 69.7702 | Great Rann of Kutch, India (10 km) |
| 140 | S2A_33SWB_20240607_0_L2A | 20 m window | 37.8312, 15.0285 | 37.8189, 15.0314 | Mount Etna, Sicily Italy (9 km) |
| 141 | S2A_29UNV_20230420_0_L2A | 20 m window | 53.2161, -8.0469 | 53.2113, -8.0442 | Irish Midlands bogs, Ireland (10 km) |
| 142 | S2B_42RXR_20240603_0_L2A | 20 m window | 27.1031, 70.4029 | 27.1186, 70.3997 | Thar Desert (India) (14 km) |
| 143 | S2A_50SPG_20240425_0_L2A | 20 m window | 37.8515, 119.2082 | 37.8192, 119.2330 | Yellow River Delta, China (11 km) |
| 144 | S2B_45QYE_20241116_0_L2A | 20 m window | 21.9892, 89.0890 | 21.9553, 89.1014 | Sundarbans mangroves, Bangladesh and India (13 km) |
| 145 | S2A_10TDT_20240708_0_L2A | 20 m window | 47.7710, -123.6314 | 47.7679, -123.6259 | Olympic Mountains, Washington, USA (4 km) |
| 146 | S2B_27VXL_20240629_0_L2A | 20 m window | 63.9767, -17.2141 | 63.9597, -17.2098 | Skeiðarársandur (Iceland) (5 km) |
| 147 | S2B_49KGP_20240809_0_L2A | 20 m window | -24.0034, 113.6385 | -24.0074, 113.6442 | Lake MacLeod, Carnarvon (1 km) |
| 148 | S2B_38SNG_20240706_0_L2A | 80 m | 37.6427, 45.4648 | 37.6233, 45.4449 | Lake Urmia, Iran (2 km) |
| 149 | S2A_39QXG_20240420_0_L2A | 20 m window | 24.2010, 52.8853 | 24.1976, 52.8930 | Al Gharbia sabkha coast, UAE (14 km) |
| 150 | S2A_19GBR_20241225_0_L2A | 20 m window | -40.1244, -72.2170 | -40.1479, -72.2088 | Valdivian rainforest and lakes, Chile (17 km) |
| 151 | S2A_4NFH_20230820_0_L2A | 20 m window | 1.9624, -157.4529 | 1.9673, -157.4585 | Kiritimati Atoll (12 km) |
| 152 | S2A_31TFJ_20241229_0_L2A | 20 m window | 43.5051, 4.6997 | 43.4971, 4.6974 | Salins de Giraud, Camargue (7 km) |
| 153 | S2A_48QYJ_20240305_0_L2A | 20 m window | 20.8971, 107.0491 | 20.8342, 107.0422 | Ha Long Bay, Vietnam (6 km) |
| 154 | S2B_52KDA_20240314_0_L2A | 20 m window | -22.5852, 128.7455 | -22.6141, 128.6778 | Lake Mackay, Australia (11 km) |
| 155 | S2B_31UFU_20240812_0_L2A | 20 m window | 52.6182, 5.6392 | 52.6053, 5.6079 | Flevoland polders, Netherlands (7 km) |
| 156 | S2B_33PVQ_20241113_0_L2A | 80 m | 13.0832, 14.8351 | 13.0717, 14.8521 |  |
| 157 | S2A_11SQR_20240620_0_L2A | 20 m window | 31.6657, -114.6417 | 31.6631, -114.6538 | Colorado River Delta, Gulf of California (10 km) |
| 158 | S2A_37MCS_20230128_0_L2A | 80 m | -2.9682, 37.7016 | -3.0262, 37.5842 |  |
| 159 | S2A_32SPD_20240204_0_L2A | 20 m window | 34.7887, 11.2764 | 34.7839, 11.2829 | Kerkennah Islands, Tunisia (12 km) |
| 160 | S2B_31TCF_20240424_0_L2A | 20 m window | 40.7163, 0.8616 | 40.7246, 0.8709 | Ebro Delta, Spain (6 km) |
| 161 | S2A_33XWG_20240718_2_L2A | 20 m window | 78.1580, 17.2960 | 78.1645, 17.3118 | Nordenskiold Land, Svalbard Norway (19 km) |
| 162 | S2A_31TFJ_20241229_0_L2A | 20 m window | 43.3756, 4.6001 | 43.3715, 4.6262 | Camargue salt pans, France (8 km) |
| 163 | S2B_47UNS_20240818_0_L2A | 80 m | 51.3476, 99.9933 | 51.3283, 99.9738 |  |
| 164 | S2A_20TLR_20240616_0_L2A | 20 m window | 45.3258, -64.1940 | 45.3392, -64.2722 | Minas Basin, Bay of Fundy (11 km) |
| 165 | S2B_45QYE_20241116_0_L2A | 80 m | 21.7197, 89.7377 | 21.9179, 89.5726 |  |
| 166 | S2B_49MHM_20230619_0_L2A | 20 m window | -8.0549, 114.1786 | -8.0585, 114.1752 | Kawah Ijen, Java (7 km) |
| 167 | S2B_28QCH_20240605_0_L2A | 80 m | 20.4595, -16.1427 | 20.6234, -16.7766 |  |
| 168 | S2A_23LMG_20240508_0_L2A | 20 m window | -12.1792, -45.8682 | -12.1892, -45.8710 | Western Bahia Cerrado soy fields, Brazil (4 km) |
| 169 | S2A_43RDP_20241003_0_L2A | 80 m | 30.6964, 74.1998 | 30.6902, 74.2747 |  |
| 170 | S2A_5QKB_20230112_0_L2A | 20 m window | 19.3441, -155.1828 | 19.3602, -155.1338 | Kilauea lava flows, Hawaii (8 km) |
| 171 | S2B_36RTV_20240724_0_L2A | 80 m | 30.7797, 30.4582 | 30.7775, 30.4354 | Western Nile Delta edge, Egypt (13 km) |
| 172 | S2A_10TES_20240625_0_L2A | 20 m window | 46.1514, -122.0278 | 46.1551, -122.0312 | Mount St Helens, USA (13 km) |
| 173 | S2B_31TCF_20240424_0_L2A | 20 m window | 40.7026, 0.8137 | 40.6969, 0.8074 | Ebro Delta, Spain (1 km) |
| 174 | S2B_46QGD_20241120_0_L2A | 20 m window | 16.3826, 95.1367 | 16.3802, 95.1423 | Irrawaddy Delta, Myanmar (21 km) |
| 175 | S2A_32SPD_20240204_0_L2A | 20 m window | 34.7519, 11.1655 | 34.7632, 11.1740 | Kerkennah Islands, Tunisia (7 km) |
| 176 | S2B_48PXR_20230313_0_L2A | 20 m window | 9.4902, 106.5628 | 9.5055, 106.5412 | Mekong River mouths, Vietnam (13 km) |
| 177 | S2A_40TFR_20241007_0_L2A | 80 m | 45.5874, 58.5016 | 45.5274, 58.7989 |  |
| 178 | S2B_32MNC_20240818_0_L2A | 80 m | -2.4370, 9.8829 | -2.4864, 9.8257 | Loango coast, Gabon (25 km) |
| 179 | S2B_52KDA_20240314_0_L2A | 20 m window | -22.4605, 128.6915 | -22.4609, 128.7017 | Lake Mackay, Australia (4 km) |
| 180 | S2A_32SPD_20240204_0_L2A | 80 m | 34.6971, 11.2213 | 34.7014, 11.1963 | Kerkennah Islands, Tunisia (1 km) |
| 181 | S2B_31UFV_20240514_0_L2A | 20 m window | 53.2471, 5.4607 | 53.2986, 5.4603 | Wadden Sea near Terschelling, Netherlands (11 km) |
| 182 | S2A_51LWC_20241021_0_L2A | 80 m | -15.5175, 123.6096 | -15.5235, 123.6214 |  |
| 183 | S2A_20TMS_20240616_0_L2A | 80 m | 46.1772, -64.2120 | 46.1535, -64.2188 |  |
| 184 | S2A_43TEM_20240621_0_L2A | 20 m window | 46.5675, 75.6646 | 46.5414, 75.5751 | Lake Balkhash, Kazakhstan (10 km) |
| 185 | S2A_21MXT_20241129_0_L2A | 20 m window | -1.9889, -55.5071 | -1.9936, -55.4963 | Amazon River near Obidos, Brazil (10 km) |
| 186 | S2B_36TXR_20240714_0_L2A | 80 m | 45.7313, 34.7445 | 45.7284, 34.7154 | Sivash lagoons, Crimea (4 km) |
| 187 | S2A_53HLE_20240110_0_L2A | 80 m | -32.1741, 133.4942 | -32.1657, 133.4726 |  |
| 188 | S2B_16QDE_20240410_0_L2A | 20 m window | 17.3229, -87.8151 | 17.3254, -87.8056 | Turneffe Atoll, Belize (5 km) |
| 189 | S2B_52KDA_20240314_0_L2A | 20 m window | -22.5602, 128.5916 | -22.5604, 128.5826 | Lake Mackay (Australia) (7 km) |
| 190 | S2A_52LDJ_20240421_0_L2A | 80 m | -14.8042, 128.7255 | -14.5776, 128.4791 |  |
| 191 | S2B_32SPD_20240718_0_L2A | 20 m window | 34.6455, 11.1628 | 34.6512, 11.1432 | Kerkennah Islands, Tunisia (7 km) |
| 192 | S2B_48PXR_20230313_0_L2A | 80 m | 9.5583, 106.7945 | 9.7316, 106.7272 | Mekong River mouths, Vietnam (29 km) |
| 193 | S2A_39TUL_20240701_0_L2A | 80 m | 45.3189, 49.1330 | 45.2360, 49.3255 |  |
| 194 | S2A_59GMM_20240220_0_L2A | 80 m | -44.0163, 169.8562 | -44.0035, 169.8373 | Lake Pukaki, New Zealand (25 km) |
| 195 | S2B_11RQL_20240327_0_L2A | 20 m window | 27.7581, -114.0517 | 27.7686, -114.0137 | Laguna Ojo de Liebre and Guerrero Negro saltworks, Baja California (4 km) |
| 196 | S2B_31UFU_20240812_0_L2A | 20 m window | 52.4597, 5.7164 | 52.4718, 5.7069 | Flevoland polders, Netherlands (12 km) |
| 197 | S2A_52LDJ_20240421_0_L2A | 80 m | -14.7970, 128.3154 | -14.7631, 128.4962 | Cambridge Gulf (Australia) (29 km) |
| 198 | S2B_52LDJ_20240903_0_L2A | 20 m window | -14.8331, 128.2480 | -15.0259, 128.1993 | Cambridge Gulf (Australia) (3 km) |
| 199 | S2B_22KGU_20240114_0_L2A | 80 m | -23.6286, -47.9924 | -23.6243, -47.9706 |  |
| 200 | S2A_51LWC_20241021_0_L2A | 80 m | -16.2740, 123.3763 | -16.2748, 123.4126 | Buccaneer Archipelago, Kimberley Australia (3 km) |
| 201 | S2A_40TFR_20241007_0_L2A | 80 m | 45.5814, 58.4866 | 45.5071, 58.8135 |  |
| 202 | S2B_16QDE_20240410_0_L2A | 20 m window | 17.4013, -87.9014 | 17.3927, -87.9077 | Turneffe Atoll, Belize (8 km) |
| 203 | S2B_28QDM_20240605_0_L2A | 20 m window | 23.9336, -15.7896 | 23.9231, -15.7935 | Dakhla lagoon, Western Sahara (10 km) |
| 204 | S2A_46RBT_20240309_0_L2A | 20 m window | 28.8466, 90.7776 | 28.8655, 90.7660 | Yamdrok Tso, Tibet (13 km) |
| 205 | S2B_50VNJ_20230617_0_L2A | 80 m | 57.2034, 118.3399 | 57.1987, 118.4074 |  |
| 206 | S2A_34KFD_20240214_0_L2A | 80 m | -19.0992, 22.1459 | -19.1319, 22.0772 |  |
| 207 | S2A_32TQS_20230910_1_L2A | 80 m | 46.1449, 12.5531 | 46.1068, 12.9431 |  |
| 208 | S2A_32TQS_20230910_1_L2A | 20 m window | 46.3803, 11.7693 | 46.4030, 11.9005 | Dolomites, Italy (12 km) |
| 209 | S2A_18FXJ_20231208_0_L2A | 20 m window | -50.9278, -72.9852 | -50.9243, -72.9877 | Torres del Paine, Chile (8 km) |
| 210 | S2A_39QXG_20240420_0_L2A | 80 m | 23.9988, 52.3978 | 24.0064, 52.3054 |  |
| 211 | S2A_38LPQ_20240907_0_L2A | 20 m window | -9.4652, 46.3422 | -9.4619, 46.3739 | Aldabra Atoll, Seychelles (5 km) |
| 212 | S2A_59GMM_20240220_0_L2A | 20 m window | -43.5890, 170.2166 | -43.5719, 170.2486 | Aoraki / Mount Cook, New Zealand (7 km) |
| 213 | S2A_59GMM_20240220_0_L2A | 20 m window | -43.6249, 170.2758 | -43.6186, 170.2729 | Aoraki / Mount Cook, New Zealand (10 km) |
| 214 | S2A_46WEB_20240913_0_L2A | 80 m | 68.9832, 93.3568 | 68.9425, 93.4433 | Putorana Plateau, Siberia Russia (24 km) |
| 215 | S2A_40RDR_20240527_0_L2A | 80 m | 27.1696, 56.3741 | 27.1228, 56.4116 | Hormuz Island, Iran (12 km) |
| 216 | S2A_43TEM_20241128_0_L2A | 20 m window | 46.6854, 75.5698 | 46.6752, 75.5545 | Lake Balkhash, Kazakhstan (10 km) |
| 217 | S2A_37PFR_20240909_0_L2A | 80 m | 13.6222, 40.7309 | 13.8803, 40.5151 | Erta Ale volcano, Ethiopia (18 km) |
| 218 | S2A_43RDP_20241003_0_L2A | 20 m window | 30.4276, 74.6695 | 30.4384, 74.6783 | Punjab farmland, India (8 km) |
| 219 | S2B_32SPD_20240718_0_L2A | 20 m window | 34.8007, 11.1698 | 34.8063, 11.2338 | Kerkennah Islands, Tunisia (12 km) |
| 220 | S2B_14TKM_20241004_0_L2A | 80 m | 42.1007, -102.4760 | 42.0882, -102.4930 |  |
| 221 | S2B_26WPE_20240717_0_L2A | 20 m window | 71.3915, -23.2306 | 71.3773, -23.3814 | Jameson Land, East Greenland (12 km) |
| 222 | S2A_38SNG_20241108_0_L2A | 20 m window | 37.3356, 45.5835 | 37.3354, 45.6024 | Lake Urmia, Iran (7 km) |
| 223 | S2A_33SWB_20241015_0_L2A | 80 m | 37.6018, 15.2055 | 37.7523, 15.1282 | Mount Etna, Sicily Italy (17 km) |
| 224 | S2A_4WEV_20240831_1_L2A | 80 m | 67.5072, -156.5187 | 67.5191, -156.5852 |  |
| 225 | S2A_38SLH_20240428_0_L2A | 80 m | 38.2294, 42.7709 | 38.1274, 42.8414 |  |
| 226 | S2A_22WEV_20240812_0_L2A | 20 m window | 67.0626, -50.0814 | 67.0361, -50.3041 | Kangerlussuaq ice margin, Greenland (7 km) |
| 227 | S2A_22MEE_20240825_0_L2A | 20 m window | -0.2979, -50.0408 | -0.2828, -50.0328 | Amazon mouth north of Marajo, Brazil (12 km) |
| 228 | S2A_36MZD_20240303_0_L2A | 80 m | -1.3045, 36.3838 | -0.9290, 36.0342 |  |
| 229 | S2A_38LPQ_20240530_0_L2A | 20 m window | -9.4795, 46.3158 | -9.4819, 46.3103 | Aldabra Atoll, Seychelles (8 km) |
| 230 | S2A_58GGR_20241216_0_L2A | 20 m window | -44.6873, 167.7899 | -44.6827, 167.7912 | Milford Sound, Fiordland (13 km) |
| 231 | S2A_19LBE_20240528_0_L2A | 20 m window | -13.8187, -71.3278 | -13.8290, -71.3622 | Vinicunca (Rainbow Mountain), Cusco Peru (7 km) |
| 232 | S2A_49KGP_20240107_0_L2A | 20 m window | -24.0198, 113.6268 | -24.0034, 113.6156 | Lake MacLeod, Carnarvon (3 km) |
| 233 | S2B_45QYE_20240430_0_L2A | 80 m | 22.0903, 89.1248 | 22.1991, 88.9611 |  |
| 234 | S2A_38LPQ_20240530_0_L2A | 20 m window | -9.4545, 46.4668 | -9.4537, 46.4599 | Aldabra Atoll, Seychelles (13 km) |
| 235 | S2A_16XDP_20240706_1_L2A | 80 m | 80.1098, -88.4994 | 80.0721, -88.5884 |  |
| 236 | S2B_50RMT_20241106_0_L2A | 80 m | 29.6915, 116.2632 | 29.7131, 116.2473 |  |
| 237 | S2B_42RXR_20241001_0_L2A | 20 m window | 27.2946, 70.3764 | 27.2860, 70.3777 | Thar Desert (India) (13 km) |
| 238 | S2A_35KLT_20240122_0_L2A | 80 m | -19.9191, 26.0394 | -19.9397, 26.0274 |  |
| 239 | S2A_38SNG_20241108_0_L2A | 80 m | 37.4828, 45.6153 | 37.5281, 45.7694 | Lake Urmia, Iran (14 km) |
| 240 | S2A_10TES_20240625_0_L2A | 20 m window | 46.2395, -122.1638 | 46.2536, -122.1607 | Mount St Helens, USA (5 km) |
| 241 | S2A_17RNH_20231101_0_L2A | 20 m window | 25.0760, -80.6951 | 25.0831, -80.6985 | Florida Bay, USA (10 km) |
| 242 | S2B_46SFF_20240818_0_L2A | 20 m window | 36.8903, 95.0972 | 36.8088, 95.2506 | Qarhan potash evaporation ponds, Qinghai, China (2 km) |
| 243 | S2A_43RDP_20241003_0_L2A | 80 m | 30.1202, 74.6325 | 30.1136, 74.6663 |  |
| 244 | S2B_18GXP_20230619_0_L2A | 20 m window | -46.5438, -72.7554 | -46.5285, -72.7301 | Lago General Carrera, Chile (8 km) |
| 245 | S2A_58GGR_20241216_0_L2A | 20 m window | -44.5055, 168.0307 | -44.5138, 167.9977 | Milford Sound, Fiordland (14 km) |
| 246 | S2A_48QYJ_20241011_0_L2A | 80 m | 20.9853, 107.0370 | 20.8999, 107.0518 | Ha Long Bay, Vietnam (12 km) |
| 247 | S2A_12TWQ_20240411_0_L2A | 80 m | 45.1131, -110.4896 | 45.1403, -110.4641 |  |
| 248 | S2A_56JNT_20240826_0_L2A | 20 m window | -25.2692, 153.0376 | -25.2668, 153.0410 | K'gari (Fraser Island) (Australia) (6 km) |
| 249 | S2A_37TGJ_20240819_0_L2A | 20 m window | 43.4454, 42.3130 | 43.4463, 42.3066 | Mount Elbrus, Caucasus Russia (16 km) |
| 250 | S2B_40RDR_20240909_0_L2A | 20 m window | 27.1533, 56.5642 | 27.1360, 56.5764 | Hormuz Island, Iran (14 km) |
| 251 | S2A_33SWB_20241015_0_L2A | 20 m window | 37.8005, 15.0331 | 37.8673, 15.0268 | Mount Etna, Sicily Italy (10 km) |
| 252 | S2A_4NFH_20230820_0_L2A | 20 m window | 1.8014, -157.4039 | 1.8002, -157.4107 | Kiritimati Atoll (8 km) |
| 253 | S2B_29SQB_20240220_0_L2A | 80 m | 36.9604, -6.1906 | 36.9952, -6.4316 |  |
| 254 | S2A_39TUL_20240701_0_L2A | 80 m | 45.3332, 49.6379 | 45.0618, 49.5802 |  |
| 255 | S2B_50SPG_20240907_0_L2A | 80 m | 37.5115, 119.0511 | 37.2543, 119.3549 |  |
| 256 | S2A_36MYB_20230210_0_L2A | 80 m | -3.6591, 35.1881 | -3.6738, 35.1803 |  |
| 257 | S2A_23LMG_20240508_0_L2A | 20 m window | -12.1475, -45.7251 | -12.1219, -45.7263 | Western Bahia Cerrado soy fields, Brazil (20 km) |
| 258 | S2B_36RTV_20240326_0_L2A | 80 m | 30.7746, 30.0243 | 30.7560, 30.0152 |  |
| 259 | S2A_19GBR_20241225_0_L2A | 20 m window | -40.1350, -72.2576 | -40.1361, -72.2500 | Valdivian rainforest and lakes, Chile (14 km) |
| 260 | S2A_17RNH_20231101_0_L2A | 80 m | 24.9770, -80.5810 | 24.9602, -80.5580 | Florida Bay, USA (19 km) |
| 261 | S2A_35TPL_20240705_0_L2A | 20 m window | 45.2057, 29.4382 | 45.2134, 29.4472 | Danube Delta, Romania (8 km) |
| 262 | S2A_43TEM_20241128_0_L2A | 20 m window | 46.6754, 75.5887 | 46.6784, 75.5969 | Lake Balkhash, Kazakhstan (11 km) |
| 263 | S2A_43TEM_20241128_0_L2A | 80 m | 46.7771, 76.3314 | 46.7203, 76.3183 |  |
| 264 | S2A_43TEM_20240621_0_L2A | 80 m | 46.5235, 75.0546 | 46.4389, 75.0819 | Lake Balkhash west, Kazakhstan (15 km) |
| 265 | S2A_39TUL_20240701_0_L2A | 80 m | 45.7033, 48.9520 | 45.7331, 49.0233 | Volga Delta, Russia (27 km) |
| 266 | S2B_36KYB_20231106_0_L2A | 20 m window | -21.6087, 35.3655 | -21.6024, 35.3804 | Bazaruto Archipelago, Mozambique (9 km) |
| 267 | S2A_51LWC_20241021_0_L2A | 80 m | -16.2742, 123.1907 | -16.3239, 123.2939 | Buccaneer Archipelago, Kimberley (8 km) |
| 268 | S2B_33XWH_20230705_1_L2A | 80 m | 79.2369, 19.4219 | 79.1908, 19.5117 |  |
| 269 | S2B_52LDJ_20240903_0_L2A | 80 m | -14.5606, 128.7105 | -14.8468, 128.6877 |  |
| 270 | S2B_11SPS_20240229_0_L2A | 80 m | 33.1917, -115.3776 | 33.2212, -115.2484 |  |
| 271 | S2B_18LUK_20241225_0_L2A | 20 m window | -13.9267, -76.2792 | -13.9403, -76.2820 | Paracas Peninsula (Peru) (10 km) |
| 272 | S2B_58KFA_20241004_0_L2A | 20 m window | -22.1585, 166.9592 | -22.1602, 166.9533 | Goro nickel laterite, New Caledonia (15 km) |
| 273 | S2A_37PFR_20240909_0_L2A | 20 m window | 13.5551, 40.7287 | 13.5549, 40.7175 | Erta Ale volcano, Ethiopia (8 km) |
| 274 | S2B_13TGG_20240606_0_L2A | 20 m window | 42.0965, -101.4504 | 42.0917, -101.4389 | Nebraska Sand Hills, USA (4 km) |
| 275 | S2B_31UFV_20240514_0_L2A | 20 m window | 53.3719, 5.4139 | 53.2856, 5.4082 | Wadden Sea near Terschelling, Netherlands (5 km) |
| 276 | S2B_17SKV_20240910_0_L2A | 20 m window | 35.5214, -83.4187 | 35.5709, -83.6285 | Great Smoky Mountains, USA (6 km) |
| 277 | S2A_17RNH_20231101_0_L2A | 20 m window | 24.9573, -80.7934 | 24.9667, -80.7505 | Florida Bay and Upper Keys (3 km) |
| 278 | S2B_17SKV_20240910_0_L2A | 80 m | 35.7808, -83.8384 | 35.8112, -83.8551 |  |
| 279 | S2A_20TLR_20240616_0_L2A | 20 m window | 45.2233, -64.4610 | 45.2240, -64.4530 | Minas Basin, Bay of Fundy (24 km) |
| 280 | S2B_38KME_20240326_0_L2A | 20 m window | -18.8418, 44.6906 | -18.8351, 44.6857 | Tsingy de Bemaraha, Madagascar (12 km) |
| 281 | S2B_31UFU_20240812_0_L2A | 20 m window | 52.5923, 5.8006 | 52.6191, 5.5033 | Flevoland polders, Netherlands (7 km) |
| 282 | S2B_36TXR_20240714_0_L2A | 20 m window | 45.8678, 34.7504 | 45.8552, 34.6961 | Sivash lagoons, Crimea (16 km) |
| 283 | S2A_22MEE_20240825_0_L2A | 20 m window | -0.2572, -50.0511 | -0.2492, -50.0394 | Amazon mouth north of Marajo, Brazil (8 km) |
| 284 | S2A_39TYF_20230122_0_L2A | 20 m window | 41.3226, 53.4535 | 41.3555, 53.4521 | Kara-Bogaz-Gol, Turkmenistan (6 km) |
| 285 | S2B_13TGG_20240606_0_L2A | 20 m window | 42.0046, -101.3121 | 42.0750, -101.2818 | Nebraska Sand Hills, USA (11 km) |
| 286 | S2B_45QYE_20240430_0_L2A | 20 m window | 21.8909, 89.3070 | 21.9222, 89.2937 | Sundarbans mangroves, Bangladesh and India (10 km) |
| 287 | S2A_10TDT_20240708_0_L2A | 80 m | 47.7050, -123.6065 | 47.4687, -124.0785 | Olympic Mountains, Washington, USA (28 km) |
| 288 | S2A_56JNT_20240826_0_L2A | 20 m window | -25.1404, 153.0974 | -25.1364, 153.1280 | K'gari (Fraser Island) (Australia) (12 km) |
| 289 | S2A_16QBF_20240329_0_L2A | 80 m | 18.4951, -89.7999 | 18.4874, -89.6941 |  |
| 290 | S2B_13SDT_20240904_0_L2A | 80 m | 33.4968, -105.8417 | 33.6078, -105.6578 | Carrizozo Malpais lava flow, New Mexico USA (25 km) |
| 291 | S2A_22WEV_20240812_0_L2A | 20 m window | 67.0388, -50.2540 | 67.0420, -50.2432 | Kangerlussuaq (Greenland) (7 km) |
| 292 | S2A_31NHE_20231213_0_L2A | 20 m window | 4.3971, 6.0900 | 4.3858, 6.0972 | Niger Delta mangroves, Nigeria (10 km) |
| 293 | S2A_36NZK_20241231_0_L2A | 20 m window | 3.5691, 36.0973 | 3.5870, 36.0886 | Lake Turkana, Kenya (3 km) |
| 294 | S2A_20TLR_20240616_0_L2A | 80 m | 45.2563, -64.2769 | 45.1178, -64.2369 | Minas Basin, Bay of Fundy (11 km) |
| 295 | S2A_19JFG_20240419_0_L2A | 80 m | -30.6323, -67.8253 | -30.4775, -67.6481 |  |
| 296 | S2B_36RTV_20240724_0_L2A | 20 m window | 30.8524, 30.4833 | 30.8155, 30.4171 | Western Nile Delta edge, Egypt (10 km) |
| 297 | S2B_21KVA_20240922_0_L2A | 20 m window | -17.7916, -57.2800 | -17.6931, -57.0890 | Paraguay River Pantanal wetlands, Brazil (7 km) |
| 298 | S2A_28WDS_20240614_0_L2A | 20 m window | 64.1537, -16.1290 | 64.1298, -16.1024 | Breidamerkurjokull and Jokulsarlon, Iceland (9 km) |
| 299 | S2A_37TGJ_20240819_0_L2A | 20 m window | 43.4731, 42.3933 | 43.4701, 42.5070 | Mount Elbrus, Caucasus Russia (14 km) |
| 300 | S2B_13TGG_20240606_0_L2A | 20 m window | 42.0906, -101.5016 | 42.0930, -101.5460 | Nebraska Sand Hills, USA (10 km) |
| 301 | S2B_19UES_20240829_0_L2A | 20 m window | 51.3759, -68.7866 | 51.3815, -68.8767 | Manicouagan Reservoir, Quebec, Canada (9 km) |
| 302 | S2B_16RBT_20241122_0_L2A | 20 m window | 29.0782, -89.3367 | 29.1090, -89.3159 | Mississippi bird's-foot delta, USA (10 km) |
| 303 | S2A_49MGM_20230624_0_L2A | 80 m | -7.7808, 113.4400 | -7.8235, 113.3772 |  |
| 304 | S2B_12VVJ_20230829_0_L2A | 80 m | 56.9393, -111.0698 | 56.7684, -111.3178 | Athabasca oil sands mines, Alberta Canada (27 km) |
| 305 | S2A_17SKV_20240329_0_L2A | 20 m window | 35.5790, -83.4871 | 35.5647, -83.4720 | Great Smoky Mountains, USA (4 km) |
| 306 | S2A_27VXL_20241012_0_L2A | 80 m | 63.7049, -17.7802 | 63.7561, -17.7174 | Eldhraun moss-covered lava, Iceland (22 km) |
| 307 | S2B_12UVB_20240721_0_L2A | 80 m | 51.1045, -111.1384 | 51.0735, -111.1274 |  |
| 308 | S2B_51XXA_20240710_0_L2A | 80 m | 72.6224, 127.9821 | 72.6042, 128.0450 |  |
| 309 | S2B_12SVH_20241003_0_L2A | 80 m | 38.7594, -111.5957 | 38.7353, -111.6237 |  |
| 310 | S2A_31TFJ_20240801_0_L2A | 80 m | 43.5086, 4.3552 | 43.5049, 4.4023 | Camargue salt pans, France (19 km) |
| 311 | S2A_10TEN_20241003_0_L2A | 80 m | 43.1106, -122.1879 | 43.0720, -122.1566 | Crater Lake, Oregon, USA (18 km) |
| 312 | S2A_38LPH_20240507_0_L2A | 80 m | -15.8697, 45.9918 | -15.8696, 46.0097 | Betsiboka Delta, Madagascar (27 km) |
| 313 | S2A_55GDN_20240309_0_L2A | 80 m | -42.6536, 146.2491 | -42.6670, 146.3179 |  |
| 314 | S2B_28RFT_20240715_0_L2A | 20 m window | 29.0032, -13.7805 | 29.0000, -13.7702 | Timanfaya, Lanzarote Spain (2 km) |
| 315 | S2A_8WNA_20240814_0_L2A | 20 m window | 68.4039, -134.4310 | 68.3849, -134.6201 | Mackenzie Delta, Canada (12 km) |
| 316 | S2A_37MCS_20230128_0_L2A | 20 m window | -2.9771, 37.4094 | -3.0539, 37.4291 | Kilimanjaro, Tanzania (10 km) |

### New

| # | Scene | Image | Start | End | Near |
| ---: | --- | --- | --- | --- | --- |
| 0 | S2B_39TUL_20241123_0_L2A | 20 m window | 45.9271, 48.5673 | 45.9230, 48.5709 | Volga Delta, Russia (13 km) |
| 1 | S2B_19LDC_20240808_0_L2A | 80 m | -15.9099, -69.7137 | -15.9045, -69.6992 |  |
| 2 | S2A_33RYH_20240611_0_L2A | 20 m window | 24.9467, 17.8587 | 24.9422, 17.8189 | Waw an Namus, Libya (7 km) |
| 3 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.9478, 17.7962 | 24.9631, 17.8564 | Waw an Namus, Libya (7 km) |
| 4 | S2A_19JFG_20240419_0_L2A | 20 m window | -29.9423, -67.8008 | -29.9152, -67.8518 | Ischigualasto / Talampaya, Argentina (8 km) |
| 5 | S2B_38LPH_20240919_0_L2A | 20 m window | -15.7973, 46.2616 | -15.7371, 46.2585 | Betsiboka Delta, Madagascar (9 km) |
| 6 | S2A_49JGM_20240203_0_L2A | 20 m window | -26.0170, 113.8801 | -26.0135, 113.8835 | Faure Sill, Shark Bay, Australia (13 km) |
| 7 | S2B_19KFP_20240407_0_L2A | 80 m | -23.8179, -67.3751 | -23.8613, -67.4048 |  |
| 8 | S2A_34HFK_20241117_0_L2A | 20 m window | -32.2856, 22.5976 | -32.2818, 22.5866 | Great Karoo, South Africa (9 km) |
| 9 | S2B_38UPU_20240907_0_L2A | 20 m window | 48.2428, 46.9996 | 48.1886, 47.0365 | Lake Baskunchak, Russia (10 km) |
| 10 | S2A_28WDT_20230531_0_L2A | 20 m window | 65.6716, -16.7693 | 65.4933, -16.6053 | Myvatn lava and lake, Iceland (14 km) |
| 11 | S2B_49KGR_20241018_0_L2A | 20 m window | -22.5484, 113.5312 | -22.4371, 113.6650 | Ningaloo Reef, Australia (13 km) |
| 12 | S2B_40RDR_20240909_0_L2A | 80 m | 27.1165, 56.1598 | 27.1573, 56.2013 | Hormuz Island, Iran (29 km) |
| 13 | S2B_40RDR_20240909_0_L2A | 20 m window | 27.1020, 56.5830 | 27.2610, 56.5751 | Hormuz Island, Iran (18 km) |
| 14 | S2A_33KVS_20240304_0_L2A | 80 m | -21.4359, 14.2591 | -21.4396, 14.2052 |  |
| 15 | S2B_12SUE_20240615_0_L2A | 80 m | 35.8100, -112.9549 | 35.8520, -112.9073 |  |
| 16 | S2A_20GMU_20241226_0_L2A | 20 m window | -42.4425, -63.9669 | -42.4326, -63.9770 | Península Valdés (Argentina) (7 km) |
| 17 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.9232, 17.8272 | 24.9849, 17.8301 | Waw an Namus, Libya (7 km) |
| 18 | S2A_30SWG_20240621_0_L2A | 20 m window | 37.1707, -2.3067 | 37.1764, -2.3003 | Tabernas Desert, Almeria Spain (16 km) |
| 19 | S2B_12VVJ_20230829_0_L2A | 80 m | 57.0120, -112.2894 | 56.7676, -112.0618 |  |
| 20 | S2B_36KYB_20231106_0_L2A | 20 m window | -21.6576, 35.5473 | -21.6642, 35.5461 | Bazaruto Archipelago, Mozambique (10 km) |
| 21 | S2A_4NFH_20230820_0_L2A | 20 m window | 1.9068, -157.2792 | 1.8674, -157.3360 | Kiritimati Atoll (10 km) |
| 22 | S2B_37PFR_20240417_0_L2A | 80 m | 14.4057, 40.2100 | 14.3894, 40.2095 | Dallol, Danakil Depression (19 km) |
| 23 | S2B_19KFQ_20240108_0_L2A | 80 m | -23.4644, -67.6058 | -23.4773, -67.5858 |  |
| 24 | S2B_27WXN_20240423_0_L2A | 80 m | 65.3813, -18.2567 | 65.3968, -18.2412 |  |
| 25 | S2B_49MHM_20231027_0_L2A | 80 m | -7.5770, 114.2393 | -7.9795, 114.6864 |  |
| 26 | S2B_26WPE_20240717_0_L2A | 20 m window | 71.4103, -23.2402 | 71.4124, -23.1939 | Jameson Land, East Greenland (16 km) |
| 27 | S2B_37PFR_20240417_0_L2A | 20 m window | 13.6213, 40.6337 | 13.6198, 40.6519 | Erta Ale volcano, Ethiopia (4 km) |
| 28 | S2A_18WXD_20240818_0_L2A | 80 m | 70.3648, -72.0353 | 70.3878, -72.0806 |  |
| 29 | S2A_59GMM_20230605_0_L2A | 20 m window | -43.9652, 170.2830 | -43.9469, 170.2779 | Lakes Pukaki and Tekapo, New Zealand (5 km) |
| 30 | S2A_37MCS_20230128_0_L2A | 80 m | -3.1321, 37.8916 | -3.1178, 37.8800 |  |
| 31 | S2A_37MCS_20230128_0_L2A | 80 m | -3.1179, 37.8800 | -3.1321, 37.8916 |  |
| 32 | S2A_19JFM_20240519_0_L2A | 20 m window | -25.7157, -67.5483 | -25.7168, -67.5545 | Salar de Antofalla, Argentina (9 km) |
| 33 | S2B_39TUL_20241123_0_L2A | 20 m window | 45.9165, 48.6881 | 45.9105, 48.6914 | Volga Delta, Russia (7 km) |
| 34 | S2A_57VXC_20240423_0_L2A | 20 m window | 56.0739, 160.6763 | 56.0720, 160.6828 | Klyuchevskaya Sopka, Kamchatka Russia (3 km) |
| 35 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.8969, 17.8466 | 24.9547, 17.8822 | Waw an Namus, Libya (10 km) |
| 36 | S2B_54TXP_20240406_0_L2A | 80 m | 43.8067, 142.4007 | 43.7865, 142.4064 |  |
| 37 | S2B_11TQJ_20240402_0_L2A | 20 m window | 43.2822, -113.6353 | 43.3697, -113.6267 | Craters of the Moon lava field, Idaho USA (7 km) |
| 38 | S2A_32VMP_20240919_0_L2A | 20 m window | 61.5991, 8.6177 | 61.5972, 8.6121 | Jotunheimen, Norway (11 km) |
| 39 | S2B_11TQJ_20240402_0_L2A | 20 m window | 43.2573, -113.4726 | 43.2509, -113.6036 | Craters of the Moon lava field, Idaho USA (11 km) |
| 40 | S2A_22WEB_20240716_0_L2A | 20 m window | 69.0533, -49.9247 | 69.0578, -49.9170 | Ilulissat Icefjord, Greenland (11 km) |
| 41 | S2B_57VXC_20241015_0_L2A | 20 m window | 55.9344, 161.0252 | 55.9412, 160.9495 | Klyuchevskaya volcano group, Kamchatka (24 km) |
| 42 | S2B_54HXF_20240422_0_L2A | 80 m | -35.8762, 142.5528 | -35.8990, 142.5359 |  |
| 43 | S2A_38UPV_20240528_0_L2A | 20 m window | 49.1761, 46.6988 | 49.1564, 46.7432 | Lake Elton, Russia (6 km) |
| 44 | S2A_50JKR_20241219_0_L2A | 80 m | -26.8705, 114.6783 | -26.9656, 114.7846 |  |
| 45 | S2B_54HXF_20240422_0_L2A | 80 m | -36.0487, 143.1252 | -36.0368, 143.0964 |  |
| 46 | S2A_40RDR_20240527_0_L2A | 20 m window | 27.0540, 56.3422 | 27.0620, 56.3370 | Hormuz Island, Iran (12 km) |
| 47 | S2A_4NFH_20230820_0_L2A | 80 m | 2.1980, -157.5288 | 2.4920, -157.1952 |  |
| 48 | S2B_49MHM_20231027_0_L2A | 80 m | -7.5331, 113.8936 | -7.5890, 114.1509 |  |
| 49 | S2B_49MHM_20231027_0_L2A | 80 m | -7.4270, 113.7552 | -7.3685, 114.3241 |  |
| 50 | S2B_32SPD_20240718_0_L2A | 80 m | 34.3406, 10.2624 | 34.3182, 10.2677 |  |
| 51 | S2A_41RQJ_20240515_0_L2A | 80 m | 25.2368, 65.3272 | 25.2535, 66.0130 |  |
| 52 | S2B_20LNP_20240901_0_L2A | 20 m window | -10.4009, -62.6739 | -10.4055, -62.6841 | Rondonia fishbone deforestation, Brazil (14 km) |
| 53 | S2A_33RYH_20240611_0_L2A | 20 m window | 24.8850, 17.8099 | 24.9016, 17.8737 | Waw an Namus, Libya (8 km) |
| 54 | S2B_57VXC_20241015_0_L2A | 20 m window | 56.1663, 160.9530 | 56.1599, 161.0041 | Klyuchevskaya Sopka, Kamchatka Russia (24 km) |
| 55 | S2A_50KPA_20241027_0_L2A | 20 m window | -22.4898, 118.4499 | -22.4824, 118.4150 | Karijini, Hamersley Range Australia (5 km) |
| 56 | S2B_36TYQ_20240621_0_L2A | 80 m | 44.2030, 36.3173 | 44.3447, 36.6142 |  |
| 57 | S2B_44SKC_20240508_0_L2A | 20 m window | 33.7471, 78.6489 | 33.7603, 78.6882 | Pangong Tso, Ladakh (2 km) |
| 58 | S2B_8WMB_20241004_0_L2A | 80 m | 68.9265, -136.9041 | 69.2127, -137.1684 |  |
| 59 | S2B_51KVB_20240929_0_L2A | 20 m window | -16.3706, 122.8215 | -16.3598, 122.8827 | Cape Leveque, Dampier Peninsula (9 km) |
| 60 | S2B_19KFP_20241113_0_L2A | 80 m | -24.0333, -67.2378 | -24.0277, -67.2604 |  |
| 61 | S2B_40RFU_20241006_0_L2A | 80 m | 30.5234, 59.1469 | 30.5152, 58.9686 | Dasht-e Lut kaluts, Iran (19 km) |
| 62 | S2B_57VXC_20241015_0_L2A | 80 m | 56.4617, 161.3611 | 56.4537, 161.4508 |  |
| 63 | S2B_57VXC_20241015_0_L2A | 80 m | 56.7040, 161.0246 | 56.8032, 161.2382 |  |
| 64 | S2A_50KNV_20240321_0_L2A | 80 m | -23.2614, 117.7055 | -23.2331, 117.7982 |  |
| 65 | S2A_49KGP_20240107_0_L2A | 80 m | -23.6989, 113.7259 | -23.8375, 113.6711 | Lake MacLeod, Carnarvon (26 km) |
| 66 | S2A_46WEB_20240913_0_L2A | 80 m | 68.8608, 93.0616 | 68.8702, 93.1114 |  |
| 67 | S2A_46WEB_20240913_0_L2A | 80 m | 69.0389, 95.3806 | 69.0554, 95.3804 |  |
| 68 | S2B_19KFR_20240308_0_L2A | 20 m window | -22.0960, -67.7489 | -22.0883, -67.7378 | Laguna Colorada, Bolivia (11 km) |
| 69 | S2A_47TQJ_20241006_1_L2A | 20 m window | 43.6853, 102.0965 | 43.6805, 102.3359 | Khongoryn Els dunes, Mongolia (11 km) |
| 70 | S2B_50KMC_20240208_0_L2A | 80 m | -20.8545, 116.3995 | -20.7627, 116.3401 |  |
| 71 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.8984, 17.7260 | 24.9512, 17.7329 | Waw an Namus (Libya) (2 km) |
| 72 | S2A_22WEV_20230311_0_L2A | 20 m window | 67.1061, -50.3541 | 67.1065, -50.3420 | Kangerlussuaq (Greenland) (7 km) |
| 73 | S2B_12VVJ_20230829_0_L2A | 80 m | 57.4084, -112.0949 | 56.9190, -111.9303 |  |
| 74 | S2B_38PKT_20240603_0_L2A | 80 m | 11.4920, 42.6762 | 11.5208, 42.5483 | Lake Assal, Djibouti (26 km) |
| 75 | S2A_38SLH_20240806_0_L2A | 20 m window | 38.5661, 42.9472 | 38.4935, 42.9818 | Lake Van, Turkey (10 km) |
| 76 | S2B_37PFR_20240417_0_L2A | 20 m window | 13.6780, 40.6918 | 13.6799, 40.6843 | Erta Ale volcano, Ethiopia (9 km) |
| 77 | S2B_49MHM_20231027_0_L2A | 80 m | -7.2842, 114.1182 | -7.7072, 114.6871 |  |
| 78 | S2B_20LNP_20240901_0_L2A | 80 m | -10.1025, -62.2246 | -10.1102, -62.2070 |  |
| 79 | S2B_20LNP_20240901_0_L2A | 20 m window | -10.1895, -62.5376 | -10.1935, -62.5534 | Rondonia fishbone deforestation, Brazil (13 km) |
| 80 | S2A_36TXR_20240331_0_L2A | 20 m window | 45.7230, 34.6848 | 45.7259, 34.6948 | Sivash lagoons, Crimea (3 km) |
| 81 | S2A_41RQJ_20240922_0_L2A | 80 m | 25.8912, 66.0397 | 25.6314, 65.4393 |  |
| 82 | S2A_37QBD_20240617_0_L2A | 20 m window | 21.0751, 37.1001 | 21.0572, 37.1007 | Dungonab Bay, Sudan (6 km) |
| 83 | S2A_4NFH_20230820_0_L2A | 80 m | 2.1372, -157.5282 | 2.4751, -157.1936 |  |
| 84 | S2A_31TGJ_20241229_0_L2A | 80 m | 43.8928, 5.7521 | 43.8499, 5.7413 | Valensole Plateau, Provence (20 km) |
| 85 | S2B_36MZC_20240616_0_L2A | 20 m window | -2.3233, 36.0314 | -2.3447, 35.9431 | Lake Natron (Tanzania) (2 km) |
| 86 | S2A_38UPV_20240925_0_L2A | 80 m | 49.5117, 47.4212 | 49.5421, 47.4451 |  |
| 87 | S2B_40SBD_20240518_0_L2A | 20 m window | 34.5934, 54.2396 | 34.5881, 54.2397 | Dasht-e Kavir (Iran) (6 km) |
| 88 | S2B_46SFF_20240818_0_L2A | 20 m window | 36.7485, 95.2631 | 36.7415, 95.2646 | Qarhan Salt Lake, Qinghai China (8 km) |
| 89 | S2B_11TQJ_20240402_0_L2A | 20 m window | 43.3129, -113.6512 | 43.3249, -113.5242 | Craters of the Moon lava field, Idaho USA (5 km) |
| 90 | S2B_30PUB_20240501_0_L2A | 20 m window | 15.1270, -4.3159 | 15.1206, -4.3260 | Inner Niger Delta near Lake Debo, Mali (15 km) |
| 91 | S2B_49KGP_20240809_0_L2A | 20 m window | -24.0277, 113.6682 | -24.0257, 113.6549 | Lake MacLeod, Carnarvon (3 km) |
| 92 | S2B_37PDQ_20241116_0_L2A | 80 m | 13.2531, 38.7026 | 13.2726, 38.6748 |  |
| 93 | S2A_19JFM_20240519_0_L2A | 20 m window | -25.6234, -67.5355 | -25.6388, -67.5452 | Salar de Antofalla, Argentina (6 km) |
| 94 | S2B_18RTN_20230226_0_L2A | 80 m | 24.4371, -77.8740 | 24.4170, -77.8556 | Andros and Tongue of the Ocean, Bahamas (19 km) |
| 95 | S2B_57VXC_20241015_0_L2A | 20 m window | 56.0611, 160.9914 | 56.0244, 160.8799 | Klyuchevskaya volcano group, Kamchatka (18 km) |
| 96 | S2B_36KYB_20231106_0_L2A | 80 m | -20.9032, 35.8927 | -20.8781, 35.9058 |  |
| 97 | S2B_42QWM_20241001_0_L2A | 20 m window | 23.9117, 70.0144 | 23.9109, 70.0198 | Rann of Kutch, India (2 km) |
| 98 | S2A_48UXE_20240913_0_L2A | 80 m | 53.5487, 107.4058 | 53.5869, 107.3725 |  |
| 99 | S2B_50KMC_20240208_0_L2A | 20 m window | -20.6891, 116.9086 | -20.7032, 116.9401 | Dampier salt ponds, Pilbara (10 km) |
| 100 | S2B_12SUG_20241003_0_L2A | 20 m window | 37.1910, -113.1017 | 37.1952, -113.0782 | Zion Canyon, Utah USA (10 km) |
| 101 | S2B_11TQJ_20240830_0_L2A | 20 m window | 43.2879, -113.5059 | 43.2898, -113.5152 | Craters of the Moon lava field, Idaho USA (8 km) |
| 102 | S2B_36MXC_20240828_0_L2A | 80 m | -2.6239, 34.3115 | -2.6257, 34.3319 |  |
| 103 | S2B_36MXC_20240828_0_L2A | 80 m | -2.6283, 34.3346 | -2.6240, 34.3112 |  |
| 104 | S2A_46WEB_20240913_0_L2A | 80 m | 69.3043, 94.0948 | 69.2808, 94.0851 |  |
| 105 | S2B_12VVJ_20230829_0_L2A | 80 m | 57.1336, -111.8963 | 56.7761, -112.0067 | Athabasca oil sands (Canada) (22 km) |
| 106 | S2B_52JFS_20240410_0_L2A | 20 m window | -25.5055, 131.0115 | -25.5082, 131.0193 | Uluru and Kata Tjuta, Australia (18 km) |
| 107 | S2B_52JFS_20240410_0_L2A | 20 m window | -25.3347, 130.8837 | -25.3489, 130.8676 | Uluru-Kata Tjuta, Australia (3 km) |
| 108 | S2B_41XMD_20240731_0_L2A | 80 m | 75.3415, 61.7837 | 75.3749, 61.6532 | Novaya Zemlya ice cap (26 km) |
| 109 | S2B_41XMD_20240731_0_L2A | 80 m | 75.3599, 62.0736 | 75.5011, 61.9848 | Novaya Zemlya ice cap (30 km) |
| 110 | S2B_8WMB_20241004_0_L2A | 20 m window | 69.0318, -134.8660 | 69.0321, -134.8818 | Mackenzie River Delta, Canada (6 km) |
| 111 | S2B_49KGR_20241018_0_L2A | 20 m window | -22.5958, 113.5260 | -22.6137, 113.6649 | Ningaloo Reef, Australia (6 km) |
| 112 | S2B_44TLP_20241008_0_L2A | 80 m | 44.0440, 78.9115 | 44.0445, 78.8746 | Aktau Mountains, Altyn-Emel Kazakhstan (3 km) |
| 113 | S2B_44TLP_20241008_0_L2A | 80 m | 44.0442, 78.9112 | 44.0444, 78.8726 | Aktau Mountains, Altyn-Emel Kazakhstan (3 km) |
| 114 | S2A_19KER_20240123_0_L2A | 80 m | -22.4786, -68.2825 | -22.4366, -68.2631 |  |
| 115 | S2A_35KLT_20240122_0_L2A | 20 m window | -20.5874, 26.0392 | -20.5940, 26.0609 | Sua Pan, Makgadikgadi Botswana (5 km) |
| 116 | S2A_37TGJ_20240401_0_L2A | 80 m | 43.5787, 41.9131 | 43.5888, 41.8828 |  |
| 117 | S2A_30TXM_20240123_0_L2A | 80 m | 41.5712, -1.2576 | 41.5979, -1.2682 |  |
| 118 | S2B_18GXP_20230619_0_L2A | 20 m window | -46.6173, -72.7510 | -46.6205, -72.7551 | Lago General Carrera, Chile (5 km) |
| 119 | S2B_49MHM_20231027_0_L2A | 80 m | -7.5583, 114.2422 | -8.0198, 114.6882 |  |
| 120 | S2A_32VMP_20240919_0_L2A | 80 m | 61.8116, 8.4217 | 61.8700, 8.3931 | Jotunheimen, Norway (27 km) |
| 121 | S2A_32VMP_20240919_0_L2A | 20 m window | 61.6737, 8.4315 | 61.6203, 8.5122 | Jotunheimen, Norway (6 km) |
| 122 | S2B_8WMB_20241004_0_L2A | 80 m | 68.9029, -135.7186 | 68.9464, -135.7818 |  |
| 123 | S2B_39TUL_20241123_0_L2A | 80 m | 45.7736, 48.5116 | 45.7704, 48.5440 | Volga Delta, Russia (16 km) |
| 124 | S2B_52JFS_20240410_0_L2A | 80 m | -25.3776, 130.1459 | -25.3868, 130.1270 |  |
| 125 | S2A_50JKR_20241219_0_L2A | 80 m | -26.6192, 114.8512 | -26.7059, 114.7722 |  |
| 126 | S2A_49KGP_20240107_0_L2A | 20 m window | -24.0486, 113.5298 | -24.0487, 113.5227 | Lake MacLeod, Carnarvon (14 km) |
| 127 | S2B_52JFS_20240410_0_L2A | 80 m | -25.7465, 131.0418 | -25.7362, 131.0169 |  |
| 128 | S2B_44TLP_20240421_0_L2A | 20 m window | 43.2932, 79.0441 | 43.2914, 79.0363 | Charyn Canyon, Kazakhstan (7 km) |
| 129 | S2A_13RGK_20240327_0_L2A | 80 m | 26.8093, -102.2989 | 26.9207, -102.3313 | Cuatro Cienegas gypsum dunes (Mexico) (21 km) |
| 130 | S2B_33XWH_20230705_1_L2A | 20 m window | 78.6745, 16.8137 | 78.6761, 16.7872 | Central Spitsbergen, Svalbard (9 km) |
| 131 | S2B_20LNP_20240901_0_L2A | 20 m window | -10.1784, -62.4946 | -10.3139, -62.4911 | Rondonia fishbone deforestation, Brazil (13 km) |
| 132 | S2B_12TVK_20241003_0_L2A | 80 m | 40.4973, -111.0132 | 40.5025, -111.0339 |  |
| 133 | S2A_43TEM_20240621_0_L2A | 80 m | 46.0661, 76.1656 | 46.1674, 75.2699 |  |
| 134 | S2A_52JFS_20241221_0_L2A | 20 m window | -25.3944, 130.9019 | -25.3801, 130.9054 | Uluru-Kata Tjuta, Australia (6 km) |
| 135 | S2A_52JFS_20241221_0_L2A | 80 m | -25.3817, 130.7371 | -25.4203, 130.7562 | Uluru-Kata Tjuta, Australia (17 km) |
| 136 | S2B_37PFR_20240417_0_L2A | 20 m window | 13.7077, 40.7619 | 13.6945, 40.7473 | Erta Ale volcano, Ethiopia (14 km) |
| 137 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.8944, 17.8503 | 24.9550, 17.8827 | Waw an Namus, Libya (10 km) |
| 138 | S2B_13RGK_20241008_0_L2A | 80 m | 27.0200, -102.1396 | 26.9912, -102.1669 | Cuatro Cienegas gypsum dunes (Mexico) (18 km) |
| 139 | S2B_42QWM_20241001_0_L2A | 20 m window | 23.8776, 69.9875 | 24.0056, 69.9804 | Rann of Kutch, India (5 km) |
| 140 | S2A_12SWJ_20240211_0_L2A | 20 m window | 38.8937, -110.8289 | 38.8907, -110.8256 | San Rafael Swell, Utah USA (8 km) |
| 141 | S2A_37TGJ_20240401_0_L2A | 80 m | 43.9044, 42.6848 | 43.4781, 42.3936 |  |
| 142 | S2A_37PFR_20240909_0_L2A | 80 m | 13.8497, 40.4763 | 13.8238, 40.4918 |  |
| 143 | S2A_21HVB_20240115_0_L2A | 80 m | -34.8926, -57.4349 | -35.0524, -57.4357 | Rio de la Plata Estuary (20 km) |
| 144 | S2B_18GXP_20230619_0_L2A | 80 m | -46.7836, -72.3866 | -46.8046, -72.4219 | Lago General Carrera, Chile-Argentina (28 km) |
| 145 | S2A_28WDT_20240919_0_L2A | 20 m window | 64.8814, -16.7418 | 65.0143, -17.0261 | Askja and Holuhraun lava field, Iceland (9 km) |
| 146 | S2A_36MYB_20230720_0_L2A | 80 m | -2.7370, 35.0162 | -2.7594, 35.0255 |  |
| 147 | S2A_19JFM_20240519_0_L2A | 20 m window | -25.6492, -67.6858 | -25.6453, -67.6820 | Salar de Antofalla, Argentina (8 km) |
| 148 | S2B_29RPP_20241007_0_L2A | 80 m | 30.4824, -6.8652 | 30.4915, -6.8485 |  |
| 149 | S2B_11SNA_20241009_0_L2A | 80 m | 36.3093, -115.7955 | 36.0570, -115.8742 |  |
| 150 | S2B_8WMB_20241004_0_L2A | 20 m window | 69.0963, -135.0363 | 69.0961, -135.0494 | Mackenzie River Delta, Canada (11 km) |
| 151 | S2B_8WMB_20241004_0_L2A | 80 m | 68.5364, -134.9065 | 68.9622, -135.4641 | Mackenzie River Delta, Canada (29 km) |
| 152 | S2A_34HFK_20241117_0_L2A | 20 m window | -32.3868, 22.4672 | -32.3841, 22.4765 | Great Karoo, South Africa (10 km) |
| 153 | S2B_19KFP_20240407_0_L2A | 20 m window | -23.6501, -67.7656 | -23.6498, -67.8184 | Lagunas Miscanti and Minique, Chile (10 km) |
| 154 | S2B_49KGR_20241018_0_L2A | 20 m window | -22.5158, 113.7241 | -22.5098, 113.7246 | Ningaloo Reef, Australia (12 km) |
| 155 | S2A_19JFG_20240419_0_L2A | 20 m window | -29.9297, -67.7770 | -29.9505, -67.8282 | Ischigualasto / Talampaya, Argentina (8 km) |
| 156 | S2B_11SNA_20240611_0_L2A | 20 m window | 36.2036, -116.9062 | 36.2108, -116.9445 | Death Valley (Badwater Basin), California USA (8 km) |
| 157 | S2B_19KFR_20240308_0_L2A | 80 m | -21.9183, -67.5618 | -21.9326, -67.5612 |  |
| 158 | S2A_38SNG_20241108_0_L2A | 20 m window | 37.3314, 45.6407 | 37.3493, 45.6514 | Lake Urmia, Iran (8 km) |
| 159 | S2B_12UVB_20240721_0_L2A | 80 m | 50.5657, -111.1036 | 50.7475, -111.0312 |  |
| 160 | S2B_19JEG_20240315_0_L2A | 80 m | -30.0101, -68.5929 | -29.9811, -68.5908 |  |
| 161 | S2A_5QKC_20241003_0_L2A | 20 m window | 19.8715, -155.4799 | 19.9102, -155.4782 | Mauna Kea summit, Hawaii USA (8 km) |
| 162 | S2B_40SBD_20240518_0_L2A | 80 m | 34.3122, 54.6325 | 34.3320, 54.6308 |  |
| 163 | S2B_11SLC_20241002_0_L2A | 80 m | 38.2986, -118.1441 | 38.2794, -118.1208 |  |
| 164 | S2B_38PKT_20240603_0_L2A | 20 m window | 11.6244, 42.3640 | 11.6261, 42.3690 | Lake Assal, Djibouti (6 km) |
| 165 | S2A_19JFG_20240419_0_L2A | 20 m window | -29.8888, -67.9107 | -29.9251, -67.8592 | Ischigualasto and Talampaya (Argentina) (10 km) |
| 166 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.9950, 17.8254 | 24.9789, 17.8264 | Waw an Namus, Libya (9 km) |
| 167 | S2B_32SPD_20240718_0_L2A | 20 m window | 34.6567, 11.1179 | 34.6574, 11.1139 | Kerkennah Islands, Tunisia (9 km) |
| 168 | S2A_5QKC_20241003_0_L2A | 20 m window | 19.8189, -155.5170 | 19.8283, -155.4945 | Mauna Kea summit, Hawaii USA (4 km) |
| 169 | S2B_12SUE_20240615_0_L2A | 80 m | 35.8749, -112.0722 | 35.7461, -112.3620 |  |
| 170 | S2A_18WXD_20240818_0_L2A | 80 m | 70.6086, -71.6884 | 70.5807, -71.6393 | Sam Ford Fjord, Baffin Island (24 km) |
| 171 | S2B_52KDA_20240920_0_L2A | 80 m | -22.5906, 128.4354 | -22.5808, 128.4412 | Lake Mackay (Australia) (19 km) |
| 172 | S2A_37PFR_20240909_0_L2A | 20 m window | 13.6939, 40.7192 | 13.6817, 40.7157 | Erta Ale volcano, Ethiopia (11 km) |
| 173 | S2B_36MXC_20240828_0_L2A | 20 m window | -2.4464, 34.6758 | -2.5266, 34.7852 | Serengeti plains, Tanzania (8 km) |
| 174 | S2A_33KXV_20240314_0_L2A | 80 m | -19.0501, 16.8979 | -18.9938, 16.9574 |  |
| 175 | S2B_38SNG_20240706_0_L2A | 20 m window | 37.4137, 45.6090 | 37.3948, 45.6094 | Lake Urmia, Iran (1 km) |
| 176 | S2B_47TQJ_20240623_0_L2A | 80 m | 43.2471, 102.3252 | 43.3118, 102.1865 |  |
| 177 | S2A_31TFJ_20241229_0_L2A | 20 m window | 43.4455, 4.5585 | 43.4426, 4.5628 | Camargue salt pans, France (3 km) |
| 178 | S2A_43TDM_20241009_0_L2A | 80 m | 46.4968, 74.2796 | 46.5120, 74.2652 |  |
| 179 | S2B_46TCK_20241003_0_L2A | 20 m window | 40.3653, 90.8232 | 40.3567, 90.8201 | Lop Nur, Xinjiang (2 km) |
| 180 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.8587, 17.7593 | 24.8506, 17.7494 | Waw an Namus (Libya) (7 km) |
| 181 | S2B_33RYH_20241004_0_L2A | 80 m | 24.8827, 17.7101 | 24.8852, 17.6825 | Waw an Namus (Libya) (7 km) |
| 182 | S2B_37PFR_20240417_0_L2A | 20 m window | 13.6678, 40.5902 | 13.6697, 40.5802 | Erta Ale volcano, Ethiopia (12 km) |
| 183 | S2A_33SWB_20241015_0_L2A | 80 m | 37.0088, 15.5648 | 37.3126, 15.4727 |  |
| 184 | S2A_13RGK_20240327_0_L2A | 80 m | 26.4766, -102.2479 | 26.4494, -102.2392 |  |
| 185 | S2B_46TCK_20241003_0_L2A | 20 m window | 40.3329, 90.7818 | 40.3448, 90.7851 | Lop Nur, Xinjiang (2 km) |
| 186 | S2A_30SWG_20240621_0_L2A | 80 m | 37.1010, -2.3820 | 37.0694, -2.3773 | Tabernas Desert, Almeria Spain (4 km) |
| 187 | S2A_33RYH_20240611_0_L2A | 20 m window | 24.8783, 17.8367 | 24.9357, 17.8764 | Waw an Namus, Libya (9 km) |
| 188 | S2A_38UPV_20240925_0_L2A | 80 m | 49.4871, 47.1476 | 49.5333, 47.2249 |  |
| 189 | S2B_47TQJ_20240623_0_L2A | 20 m window | 43.6796, 102.0313 | 43.7431, 102.0811 | Khongoryn Els dunes, Mongolia (14 km) |
| 190 | S2A_47TQJ_20241006_1_L2A | 80 m | 43.2822, 101.5409 | 43.3651, 101.7796 |  |
| 191 | S2B_19JEG_20240315_0_L2A | 80 m | -30.3815, -67.8920 | -30.4446, -67.9659 |  |
| 192 | S2B_13SCS_20240527_0_L2A | 80 m | 33.3263, -106.2177 | 33.2639, -106.1375 |  |
| 193 | S2A_13SCS_20240909_0_L2A | 80 m | 33.3483, -106.1920 | 33.1234, -106.2717 |  |
| 194 | S2B_19KFR_20240308_0_L2A | 20 m window | -22.2442, -67.7682 | -22.2827, -67.7736 | Laguna Colorada, Bolivia (7 km) |
| 195 | S2B_34QBG_20241011_0_L2A | 20 m window | 19.7937, 18.5305 | 19.6783, 18.5407 | Emi Koussi, Tibesti Chad (6 km) |
| 196 | S2A_30RTV_20240621_0_L2A | 20 m window | 30.9960, -5.7049 | 30.9965, -5.6999 | Jbel Saghro, Morocco (12 km) |
| 197 | S2A_48TUP_20241023_0_L2A | 80 m | 43.9088, 103.5226 | 44.2003, 103.3340 | Bayanzag Flaming Cliffs (Mongolia) (24 km) |
| 198 | S2A_49KGP_20240107_0_L2A | 20 m window | -24.1117, 113.7038 | -24.1142, 113.6941 | Lake MacLeod, Carnarvon (14 km) |
| 199 | S2A_33JWN_20240304_0_L2A | 80 m | -25.1493, 15.4525 | -24.8566, 15.1488 | Sossusvlei dune sea, Namibia (28 km) |
| 200 | S2B_36QVF_20240718_0_L2A | 20 m window | 18.2874, 32.7709 | 18.2844, 32.7788 | Bayuda volcanic field, Sudan (3 km) |
| 201 | S2A_48TUP_20241023_0_L2A | 20 m window | 44.0177, 103.8530 | 44.0436, 103.7582 | Flaming Cliffs (Bayanzag), Mongolia (13 km) |
| 202 | S2B_19KEQ_20240914_0_L2A | 80 m | -23.1948, -68.6821 | -23.3699, -68.7479 |  |
| 203 | S2A_47TQJ_20241006_1_L2A | 80 m | 43.5103, 101.5003 | 43.4385, 101.7050 |  |
| 204 | S2B_33JYJ_20240402_0_L2A | 20 m window | -28.2533, 17.1063 | -28.2518, 17.1108 | Richtersveld, South Africa (13 km) |
| 205 | S2A_35KLT_20240730_0_L2A | 80 m | -20.1221, 26.0098 | -20.0531, 25.9749 |  |
| 206 | S2B_54HXF_20241218_0_L2A | 80 m | -35.8803, 142.3966 | -35.8623, 142.3894 |  |
| 207 | S2A_34JDS_20241130_0_L2A | 80 m | -25.6902, 20.0315 | -25.8915, 20.4785 | Kgalagadi red dunes, Kalahari (25 km) |
| 208 | S2A_48TUP_20241023_0_L2A | 80 m | 43.2882, 102.6091 | 43.2535, 102.6157 |  |
| 209 | S2A_33JWN_20240304_0_L2A | 20 m window | -24.8690, 15.3594 | -24.8699, 15.2143 | Sossusvlei dune sea, Namibia (13 km) |
| 210 | S2A_47SND_20240330_0_L2A | 80 m | 39.6923, 100.1281 | 39.7174, 100.1483 |  |
| 211 | S2A_33JWN_20240304_0_L2A | 80 m | -24.6835, 15.1581 | -24.8813, 15.1050 | Sossusvlei dune sea, Namibia (17 km) |
| 212 | S2B_48TUP_20240521_0_L2A | 80 m | 43.9137, 102.9863 | 44.1109, 103.2304 |  |
| 213 | S2B_40REU_20241009_0_L2A | 20 m window | 30.3013, 57.9700 | 30.3469, 57.9912 | Dasht-e Lut yardangs, Iran (9 km) |
| 214 | S2B_40SBD_20240518_0_L2A | 80 m | 34.3787, 54.6278 | 34.3606, 54.6528 |  |
| 215 | S2A_48TUP_20241023_0_L2A | 20 m window | 44.1360, 103.6640 | 44.1641, 103.7247 | Bayanzag Flaming Cliffs (Mongolia) (0 km) |
| 216 | S2A_47TQJ_20241006_1_L2A | 80 m | 44.0258, 102.0012 | 44.0455, 101.9621 |  |
| 217 | S2A_48TTK_20241013_0_L2A | 80 m | 40.4170, 102.5703 | 40.2846, 101.5354 |  |
| 218 | S2A_47SND_20240330_0_L2A | 80 m | 39.0655, 99.9977 | 39.0862, 100.0273 | Zhangye Danxia, Gansu China (16 km) |
| 219 | S2B_40RFU_20241006_0_L2A | 20 m window | 30.2997, 58.9176 | 30.2573, 58.9072 | Dasht-e Lut kaluts, Iran (8 km) |
| 220 | S2B_48TUP_20240521_0_L2A | 80 m | 43.9018, 102.9856 | 43.8843, 103.0175 |  |
| 221 | S2B_40RFU_20240618_0_L2A | 20 m window | 30.3548, 59.1269 | 30.2791, 59.0996 | Dasht-e Lut kaluts, Iran (7 km) |
| 222 | S2A_30PVA_20240416_0_L2A | 20 m window | 14.2490, -3.4684 | 14.2385, -3.4640 | Bandiagara Escarpment, Mali (12 km) |
| 223 | S2A_19KER_20240919_0_L2A | 80 m | -21.9228, -68.7151 | -21.9670, -68.6494 |  |
| 224 | S2B_11SMA_20241009_0_L2A | 80 m | 36.3936, -117.5622 | 36.4156, -117.5889 |  |
| 225 | S2B_47TQJ_20240623_0_L2A | 80 m | 43.6522, 101.6352 | 43.7953, 101.6914 |  |
| 226 | S2B_40REU_20240611_0_L2A | 20 m window | 30.3159, 57.9881 | 30.3222, 57.9832 | Dasht-e Lut yardangs, Iran (9 km) |
| 227 | S2A_30RUV_20240621_0_L2A | 20 m window | 31.1173, -4.0622 | 31.2270, -4.0978 | Erg Chebbi, Morocco (8 km) |
| 228 | S2B_40RFU_20241006_0_L2A | 20 m window | 30.4498, 59.0496 | 30.4506, 59.0646 | Dasht-e Lut kaluts, Iran (11 km) |
| 229 | S2B_47TQJ_20240623_0_L2A | 20 m window | 43.6971, 102.0544 | 43.7369, 102.1109 | Khongoryn Els dunes, Mongolia (12 km) |
| 230 | S2B_46TCK_20241003_0_L2A | 20 m window | 40.2703, 90.7931 | 40.2608, 90.8254 | Lop Nur, Xinjiang (9 km) |
| 231 | S2B_44TLP_20240809_0_L2A | 20 m window | 43.3909, 79.0566 | 43.3704, 78.9866 | Charyn Canyon, Kazakhstan (6 km) |
| 232 | S2A_13RGK_20240327_0_L2A | 20 m window | 26.7403, -102.1690 | 26.7457, -102.1719 | Cuatro Cienegas gypsum dunes (Mexico) (14 km) |
| 233 | S2A_47TQJ_20241006_1_L2A | 20 m window | 43.6781, 102.0459 | 43.6933, 102.2792 | Khongoryn Els dunes, Mongolia (11 km) |
| 234 | S2A_36SXH_20241004_0_L2A | 80 m | 38.5124, 34.7587 | 38.5313, 34.7254 | Cappadocia, Turkey (17 km) |
| 235 | S2B_12TUL_20240320_0_L2A | 80 m | 40.6090, -112.8966 | 40.9204, -113.2354 |  |
| 236 | S2B_38SNG_20240706_0_L2A | 80 m | 37.8059, 45.8725 | 37.7779, 45.8928 |  |
| 237 | S2A_35KLT_20240122_0_L2A | 20 m window | -20.6039, 25.9786 | -20.5623, 25.8834 | Sua Pan, Makgadikgadi, Botswana (4 km) |
| 238 | S2B_36SWH_20241009_0_L2A | 80 m | 38.6348, 34.1468 | 38.6539, 34.1430 |  |
| 239 | S2A_12SXD_20240617_0_L2A | 80 m | 35.1315, -109.0360 | 34.9022, -109.8823 | Painted Desert / Petrified Forest, Arizona USA (30 km) |
| 240 | S2B_35SPB_20241005_0_L2A | 80 m | 37.5116, 29.1937 | 37.4813, 29.1889 |  |
| 241 | S2A_43TGJ_20240605_0_L2A | 20 m window | 44.1654, 78.5682 | 44.1394, 78.6301 | Aktau Mountains, Altyn-Emel Kazakhstan (23 km) |
| 242 | S2B_47UNS_20240818_0_L2A | 80 m | 50.6443, 99.6284 | 50.6242, 99.6181 |  |
| 243 | S2A_43SFV_20240317_0_L2A | 80 m | 35.2252, 76.4010 | 35.6947, 76.2715 |  |
| 244 | S2A_37TGJ_20240401_0_L2A | 80 m | 43.7440, 42.4189 | 43.7801, 42.4300 |  |
| 245 | S2A_28WDT_20240919_0_L2A | 80 m | 65.2918, -16.1208 | 65.2293, -16.5701 |  |
| 246 | S2B_11SLC_20241002_0_L2A | 80 m | 38.0373, -118.2117 | 37.9365, -118.2472 |  |
| 247 | S2B_11SLC_20240604_0_L2A | 20 m window | 37.8887, -118.9727 | 37.9370, -118.9284 | Mono Lake, California (11 km) |
| 248 | S2A_38SNG_20241108_0_L2A | 20 m window | 37.3342, 45.6312 | 37.3546, 45.7154 | Lake Urmia, Iran (9 km) |
| 249 | S2B_19KFR_20240308_0_L2A | 20 m window | -22.2223, -67.6553 | -22.2019, -67.6949 | Laguna Colorada, Bolivia (11 km) |
| 250 | S2A_30SWG_20240621_0_L2A | 80 m | 37.7198, -2.3138 | 37.7373, -2.2904 |  |
| 251 | S2B_11SLC_20240604_0_L2A | 20 m window | 38.1032, -119.0742 | 38.1018, -118.8906 | Mono Lake, California (11 km) |
| 252 | S2A_44SKA_20240930_0_L2A | 80 m | 32.2503, 78.8436 | 32.2297, 78.8633 |  |
| 253 | S2A_16XDP_20240706_1_L2A | 80 m | 79.1983, -91.1291 | 79.2001, -91.1900 |  |
| 254 | S2A_47TQJ_20241006_1_L2A | 20 m window | 43.7473, 102.0408 | 43.7421, 102.0470 | Khongoryn Els dunes, Mongolia (13 km) |
| 255 | S2A_43TEM_20240621_0_L2A | 80 m | 46.0994, 76.3479 | 46.1828, 75.8124 |  |
| 256 | S2A_19JFM_20240519_0_L2A | 80 m | -26.0393, -67.8028 | -26.0356, -67.8964 |  |
| 257 | S2B_40RFU_20241006_0_L2A | 20 m window | 30.2895, 59.1209 | 30.3430, 59.0726 | Dasht-e Lut kaluts, Iran (6 km) |
| 258 | S2B_44SKC_20240925_0_L2A | 80 m | 33.3486, 77.8850 | 33.3880, 77.8514 |  |
| 259 | S2A_43TEM_20241128_0_L2A | 80 m | 46.3654, 76.2283 | 46.1921, 75.6311 |  |
| 260 | S2B_19KEP_20241113_0_L2A | 80 m | -24.3133, -68.5453 | -24.3234, -68.5848 |  |
| 261 | S2B_12SUE_20240615_0_L2A | 80 m | 35.9397, -113.0060 | 36.1136, -112.8336 |  |
| 262 | S2A_47TQJ_20241006_1_L2A | 80 m | 44.0089, 102.7338 | 43.8306, 102.6120 |  |
| 263 | S2B_44TLP_20241008_0_L2A | 20 m window | 44.0715, 78.8990 | 44.0208, 78.8364 | Aktau Mountains, Altyn-Emel Kazakhstan (1 km) |
| 264 | S2B_40RFU_20240618_0_L2A | 80 m | 30.6954, 59.1387 | 30.7097, 59.1809 |  |
| 265 | S2B_40RFU_20241006_0_L2A | 20 m window | 30.3499, 58.9938 | 30.2448, 59.0793 | Dasht-e Lut kaluts, Iran (6 km) |
| 266 | S2A_30RUV_20240621_0_L2A | 20 m window | 31.1135, -4.0846 | 31.1085, -4.0944 | Erg Chebbi, Morocco (10 km) |
| 267 | S2A_43TEM_20240621_0_L2A | 80 m | 46.1255, 76.1895 | 45.9983, 75.8463 |  |
| 268 | S2A_27VXL_20241012_0_L2A | 20 m window | 63.7407, -18.0437 | 63.7288, -18.0125 | Eldhraun moss-covered lava, Iceland (9 km) |
| 269 | S2B_27VXL_20240629_0_L2A | 20 m window | 63.8393, -16.9596 | 63.9457, -16.9206 | Skeidararsandur outwash plain, Iceland (13 km) |
| 270 | S2A_40RDR_20240527_0_L2A | 80 m | 27.8552, 56.3297 | 27.8917, 56.3014 |  |
| 271 | S2B_36TYQ_20240621_0_L2A | 20 m window | 45.0654, 36.1135 | 45.0711, 36.1072 | Lake Koyashskoye, Crimea (7 km) |
| 272 | S2A_38UPV_20240925_0_L2A | 80 m | 49.4828, 46.5981 | 49.4339, 46.6246 |  |
| 273 | S2A_35KLT_20240122_0_L2A | 20 m window | -20.6380, 26.1028 | -20.6031, 26.0938 | Sua Pan, Makgadikgadi Botswana (2 km) |
| 274 | S2A_11SMA_20240616_0_L2A | 80 m | 36.7551, -117.1297 | 36.7662, -117.1130 |  |
| 275 | S2B_47TQJ_20240623_0_L2A | 20 m window | 43.6689, 102.0371 | 43.7196, 102.0657 | Khongoryn Els dunes, Mongolia (15 km) |
| 276 | S2A_12STA_20241008_0_L2A | 80 m | 32.1238, -113.1769 | 32.1134, -113.1987 |  |
| 277 | S2A_48TUP_20241023_0_L2A | 80 m | 43.8651, 102.6735 | 44.0132, 102.9656 |  |
| 278 | S2A_19JFM_20240110_0_L2A | 20 m window | -25.6813, -67.6378 | -25.6558, -67.6552 | Salar de Antofalla, Argentina (5 km) |
| 279 | S2A_19JFM_20240110_0_L2A | 20 m window | -25.6811, -67.6403 | -25.6537, -67.6522 | Salar de Antofalla, Argentina (5 km) |
| 280 | S2B_11SNA_20240611_0_L2A | 80 m | 36.2505, -116.4376 | 36.2738, -116.4467 | Badwater Basin, Death Valley (30 km) |
| 281 | S2B_48TUP_20240521_0_L2A | 20 m window | 44.1237, 103.6550 | 44.1818, 103.6037 | Bayanzag Flaming Cliffs (Mongolia) (6 km) |
| 282 | S2A_47SNA_20240330_0_L2A | 80 m | 36.8091, 99.2267 | 36.7698, 99.1771 |  |
| 283 | S2A_19JDM_20240323_0_L2A | 20 m window | -26.1824, -69.1985 | -26.2140, -69.1957 | Salar de Pedernales, Chile (11 km) |
| 284 | S2B_48TUP_20240521_0_L2A | 80 m | 43.9593, 102.9795 | 44.0179, 103.0644 |  |
| 285 | S2B_40RFU_20241006_0_L2A | 20 m window | 30.4653, 59.1076 | 30.3657, 59.0251 | Dasht-e Lut kaluts, Iran (7 km) |
| 286 | S2A_44RNV_20240417_0_L2A | 80 m | 30.9199, 81.1725 | 30.9321, 81.1956 | Mount Kailash / Lake Manasarovar, Tibet (27 km) |
| 287 | S2A_33JWN_20240304_0_L2A | 20 m window | -24.7591, 15.1894 | -24.8648, 15.1796 | Sossusvlei dune sea, Namibia (14 km) |
| 288 | S2A_48TUP_20241023_0_L2A | 20 m window | 44.1759, 103.7699 | 44.1718, 103.7792 | Flaming Cliffs (Bayanzag), Gobi (5 km) |
| 289 | S2B_40RFU_20241006_0_L2A | 80 m | 30.5651, 59.1426 | 30.5533, 58.9976 | Dasht-e Lut kaluts, Iran (23 km) |
| 290 | S2A_19JFM_20240110_0_L2A | 20 m window | -25.5463, -67.4905 | -25.5520, -67.4809 | Salar de Antofalla, Argentina (11 km) |
| 291 | S2B_53JQM_20240312_0_L2A | 80 m | -26.0189, 137.0053 | -26.1889, 137.4027 |  |
| 292 | S2A_13SCS_20240909_0_L2A | 80 m | 33.3051, -106.1839 | 33.2602, -106.2096 |  |
| 293 | S2A_34JDS_20240305_0_L2A | 20 m window | -25.8862, 20.4640 | -25.8647, 20.4833 | Kgalagadi red dunes, Kalahari (9 km) |
| 294 | S2B_19KFR_20240308_0_L2A | 80 m | -21.7393, -68.0066 | -21.7592, -67.9712 |  |
| 295 | S2A_33JWN_20240304_0_L2A | 80 m | -24.8045, 15.3630 | -25.1330, 15.2204 | Sossusvlei dune sea, Namibia (24 km) |
| 296 | S2A_11SMA_20240616_0_L2A | 80 m | 36.3983, -117.4795 | 36.4158, -117.4685 |  |
| 297 | S2A_11SMA_20240616_0_L2A | 80 m | 36.3972, -117.4805 | 36.4166, -117.4679 |  |
| 298 | S2B_38SNG_20240706_0_L2A | 20 m window | 37.4947, 45.5482 | 37.4947, 45.5363 | Lake Urmia, Iran (12 km) |
| 299 | S2B_45TYE_20241023_0_L2A | 20 m window | 40.2238, 90.5410 | 40.2009, 90.5494 | Lop Nur salt basin, China (16 km) |
| 300 | S2A_43TDM_20241009_0_L2A | 80 m | 46.7325, 73.8302 | 46.7379, 73.8852 |  |
| 301 | S2A_38UPV_20240925_0_L2A | 20 m window | 49.2483, 46.5447 | 49.2407, 46.6183 | Lake Elton, Russia (14 km) |
| 302 | S2B_12SUG_20241003_0_L2A | 80 m | 37.5556, -112.0605 | 37.5393, -112.0403 | Bryce Canyon, Utah USA (12 km) |
| 303 | S2B_19KEQ_20240914_0_L2A | 20 m window | -22.8230, -68.3776 | -22.7998, -68.3156 | Valle de la Luna / Cordillera de la Sal, Chile (14 km) |
| 304 | S2B_33JYK_20240412_0_L2A | 80 m | -27.7204, 17.2263 | -27.7046, 17.2295 |  |
| 305 | S2A_47TQJ_20241006_1_L2A | 20 m window | 43.6764, 102.0971 | 43.6804, 102.3018 | Khongoryn Els dunes, Mongolia (11 km) |
| 306 | S2B_44TLP_20241008_0_L2A | 80 m | 43.4241, 79.7609 | 43.3993, 79.7745 |  |
| 307 | S2A_45TYH_20241001_0_L2A | 80 m | 43.2525, 90.4661 | 43.1725, 90.7297 |  |
| 308 | S2B_26WNE_20240707_0_L2A | 20 m window | 71.4452, -25.8888 | 71.4427, -25.8751 | Scoresby Sund inner fjords, Greenland (15 km) |
| 309 | S2A_47TQJ_20241006_1_L2A | 20 m window | 43.6760, 102.0378 | 43.6918, 102.2838 | Khongoryn Els dunes, Mongolia (11 km) |
| 310 | S2B_40RFU_20241006_0_L2A | 80 m | 30.0393, 59.0332 | 30.0519, 59.0576 |  |
| 311 | S2B_40RFU_20240618_0_L2A | 20 m window | 30.4308, 58.9639 | 30.3908, 58.9070 | Dasht-e Lut kaluts, Iran (8 km) |
| 312 | S2A_11SQA_20240703_0_L2A | 80 m | 36.9838, -114.3986 | 36.9866, -114.3794 |  |
| 313 | S2B_47TQJ_20240623_0_L2A | 20 m window | 43.7407, 102.1221 | 43.7080, 102.2763 | Khongoryn Els dunes, Mongolia (6 km) |
| 314 | S2B_37PDQ_20240301_0_L2A | 20 m window | 13.3420, 38.2641 | 13.3272, 38.2726 | Simien Mountains, Ethiopia (12 km) |
| 315 | S2A_47TQJ_20241006_1_L2A | 20 m window | 43.6751, 102.0868 | 43.6883, 102.3243 | Khongoryn Els dunes, Mongolia (11 km) |
| 316 | S2A_12SXD_20241005_0_L2A | 20 m window | 34.9857, -109.7186 | 34.9847, -109.7254 | Painted Desert, Arizona (10 km) |
| 317 | S2A_18LZH_20240528_0_L2A | 80 m | -16.1551, -71.9538 | -16.2844, -71.9120 |  |
| 318 | S2B_38UPU_20240907_0_L2A | 20 m window | 48.1451, 46.8524 | 48.1481, 46.8437 | Lake Baskunchak, Russia (6 km) |
| 319 | S2A_45TYH_20241001_0_L2A | 80 m | 43.2391, 90.5037 | 43.1117, 90.3241 |  |
| 320 | S2A_36MZD_20240303_0_L2A | 20 m window | -1.7879, 36.3453 | -1.7916, 36.3505 | Lake Magadi, Kenya (12 km) |
| 321 | S2B_11SLC_20241002_0_L2A | 80 m | 38.3711, -119.0306 | 38.4182, -118.8980 |  |
| 322 | S2B_38UPU_20240907_0_L2A | 80 m | 48.0889, 46.7898 | 48.0486, 46.9987 | Lake Baskunchak, Russia (15 km) |
| 323 | S2A_21HVB_20240115_0_L2A | 80 m | -34.8765, -56.9154 | -35.3207, -57.1698 |  |
| 324 | S2B_19JEG_20240315_0_L2A | 80 m | -30.0337, -68.6687 | -29.9860, -68.6560 |  |
| 325 | S2A_21HVB_20240115_0_L2A | 80 m | -34.6864, -57.4291 | -35.1730, -57.2682 | Rio de la Plata Estuary (20 km) |
| 326 | S2B_11SLC_20240604_0_L2A | 20 m window | 37.8934, -118.9693 | 37.9095, -118.9507 | Mono Lake, California (12 km) |
| 327 | S2A_38SNG_20241108_0_L2A | 20 m window | 37.3982, 45.5507 | 37.4124, 45.5549 | Lake Urmia, Iran (4 km) |
| 328 | S2B_46SFF_20240818_0_L2A | 80 m | 36.9883, 94.3307 | 36.9750, 94.3423 |  |
| 329 | S2B_12TTL_20240320_0_L2A | 80 m | 40.9699, -114.2642 | 40.8836, -114.1994 |  |
| 330 | S2B_23LLG_20240930_0_L2A | 20 m window | -12.1595, -45.8674 | -12.1541, -45.8665 | Western Bahia Cerrado soy fields, Brazil (6 km) |
| 331 | S2A_20KKU_20241115_0_L2A | 20 m window | -23.7504, -65.7501 | -23.7650, -65.8154 | Salinas Grandes, Argentina (14 km) |
| 332 | S2B_54HXF_20240422_0_L2A | 20 m window | -35.3761, 142.8335 | -35.3820, 142.8340 | Lake Tyrrell, Victoria (5 km) |
| 333 | S2A_19JFM_20240519_0_L2A | 80 m | -25.5007, -67.5080 | -25.4832, -67.5127 | Salar de Antofalla, Argentina (11 km) |
| 334 | S2B_19KEP_20240427_0_L2A | 20 m window | -23.6466, -68.4178 | -23.6793, -68.4742 | Salar de Atacama lithium ponds, Chile (11 km) |
| 335 | S2B_11SLC_20241002_0_L2A | 20 m window | 37.9736, -118.8940 | 38.1012, -118.8515 | Mono Lake, California (12 km) |
| 336 | S2B_37REJ_20240602_0_L2A | 20 m window | 25.8986, 39.8445 | 25.8987, 39.8380 | Harrat Khaybar, Saudi Arabia (12 km) |
| 337 | S2B_11SNA_20241009_0_L2A | 20 m window | 36.1552, -116.7144 | 36.1632, -116.7113 | Badwater Basin, Death Valley (9 km) |
| 338 | S2B_11TQJ_20240830_0_L2A | 20 m window | 43.3429, -113.5372 | 43.3372, -113.5489 | Craters of the Moon lava field, Idaho USA (1 km) |
| 339 | S2B_11TQJ_20240830_0_L2A | 20 m window | 43.3430, -113.5413 | 43.3368, -113.5480 | Craters of the Moon lava field, Idaho USA (1 km) |
| 340 | S2B_38SNG_20240706_0_L2A | 20 m window | 37.3605, 45.6339 | 37.3654, 45.6280 | Lake Urmia, Iran (5 km) |
| 341 | S2B_27VXL_20240629_0_L2A | 20 m window | 63.7620, -17.4529 | 63.7811, -17.4688 | Skeidararsandur, Iceland (16 km) |
| 342 | S2B_54HXF_20240422_0_L2A | 80 m | -35.5184, 143.2663 | -35.5187, 143.3027 |  |
| 343 | S2B_40RFU_20240618_0_L2A | 20 m window | 30.3348, 59.1444 | 30.2379, 59.1024 | Dasht-e Lut kaluts, Iran (10 km) |
| 344 | S2B_57VXC_20241015_0_L2A | 80 m | 55.9454, 160.9202 | 55.9667, 160.8604 | Klyuchevskaya volcano group, Kamchatka (18 km) |
| 345 | S2B_10TEN_20240610_0_L2A | 80 m | 43.2603, -121.9312 | 43.2530, -121.9592 |  |
| 346 | S2A_43TEM_20240621_0_L2A | 20 m window | 46.6151, 75.4222 | 46.6159, 75.4112 | Lake Balkhash, Kazakhstan (7 km) |
| 347 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.9556, 17.8414 | 24.9728, 17.8574 | Waw an Namus, Libya (9 km) |
| 348 | S2B_12SUG_20241003_0_L2A | 80 m | 37.5350, -113.0925 | 37.5236, -113.0753 |  |
| 349 | S2A_29VPE_20230420_0_L2A | 20 m window | 57.8539, -6.8725 | 57.8575, -6.8747 | Sound of Harris, Outer Hebrides (16 km) |
| 350 | S2A_37PFR_20240909_0_L2A | 20 m window | 13.5323, 40.7169 | 13.5424, 40.7105 | Erta Ale volcano, Ethiopia (8 km) |
| 351 | S2B_51XXA_20240710_0_L2A | 20 m window | 72.8761, 126.7205 | 72.8661, 126.6968 | Lena Delta, Russia (10 km) |
| 352 | S2B_11TQJ_20240830_0_L2A | 20 m window | 43.4467, -113.6209 | 43.4417, -113.6250 | Craters of the Moon, Idaho (8 km) |
| 353 | S2A_28RCS_20241110_0_L2A | 20 m window | 28.2291, -16.6922 | 28.2270, -16.6809 | Teide caldera, Tenerife Spain (7 km) |
| 354 | S2B_26WPE_20240717_0_L2A | 20 m window | 71.3605, -23.1495 | 71.3652, -23.1659 | Jameson Land, East Greenland (14 km) |
| 355 | S2A_37MCS_20230128_0_L2A | 20 m window | -2.9795, 37.3318 | -3.0233, 37.3322 | Kilimanjaro, Tanzania (8 km) |
| 356 | S2A_30VVJ_20230226_0_L2A | 20 m window | 57.0077, -3.4460 | 57.0004, -3.4511 | Cairngorms heather moorland, Scotland (13 km) |
| 357 | S2B_44TLP_20241008_0_L2A | 80 m | 43.2794, 79.6201 | 43.4974, 79.4278 |  |
| 358 | S2B_54HXF_20240422_0_L2A | 80 m | -36.1193, 143.0365 | -36.0906, 143.0397 |  |
| 359 | S2B_11SQR_20241003_0_L2A | 20 m window | 31.6207, -114.8408 | 31.6111, -114.8589 | Colorado River Delta, Gulf of California (13 km) |
| 360 | S2A_38SLH_20240806_0_L2A | 80 m | 38.0623, 43.1561 | 38.0647, 43.1824 |  |
| 361 | S2B_40RFU_20241006_0_L2A | 20 m window | 30.4568, 59.1803 | 30.3571, 59.0247 | Dasht-e Lut kaluts, Iran (8 km) |
| 362 | S2A_49KGR_20240206_0_L2A | 20 m window | -22.5926, 113.7269 | -22.5883, 113.7320 | Ningaloo Reef, Australia (8 km) |
| 363 | S2A_38UPV_20240528_0_L2A | 20 m window | 49.1660, 46.7750 | 49.1540, 46.7717 | Lake Elton, Russia (9 km) |
| 364 | S2B_44TLP_20240809_0_L2A | 80 m | 43.5538, 78.9354 | 43.5592, 78.9986 | Charyn Canyon, Kazakhstan (25 km) |
| 365 | S2A_19JFM_20240110_0_L2A | 20 m window | -25.5589, -67.7277 | -25.5618, -67.7111 | Salar de Antofalla, Argentina (12 km) |
| 366 | S2A_20GMU_20240420_0_L2A | 80 m | -42.3167, -63.8218 | -42.3396, -63.8534 | Península Valdés (Argentina) (23 km) |
| 367 | S2B_42QWM_20241001_0_L2A | 20 m window | 24.0642, 69.9147 | 24.0664, 69.9284 | Great Rann of Kutch, India (13 km) |
| 368 | S2B_36MXC_20240828_0_L2A | 20 m window | -2.5106, 34.7461 | -2.4797, 34.7752 | Serengeti plains, Tanzania (4 km) |
| 369 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.8846, 17.8281 | 24.8960, 17.8796 | Waw an Namus, Libya (9 km) |
| 370 | S2A_21MXT_20241129_0_L2A | 80 m | -2.1258, -55.1172 | -2.1519, -55.1272 |  |
| 371 | S2B_37PFR_20240417_0_L2A | 20 m window | 13.5753, 40.6067 | 13.5133, 40.5713 | Erta Ale volcano, Ethiopia (11 km) |
| 372 | S2A_16XDP_20240706_1_L2A | 20 m window | 79.4717, -90.6580 | 79.4691, -90.6207 | Axel Heiberg Island, Nunavut Canada (4 km) |
| 373 | S2B_49MHM_20231027_0_L2A | 80 m | -8.0784, 114.5984 | -8.0046, 114.6401 |  |
| 374 | S2A_27VXL_20241012_0_L2A | 20 m window | 63.6475, -18.3008 | 63.6901, -18.3572 | Eldhraun moss-covered lava, Iceland (11 km) |
| 375 | S2A_22MEE_20240825_0_L2A | 20 m window | -0.1045, -50.2060 | -0.1143, -50.0690 | Amazon mouth north of Marajo, Brazil (11 km) |
| 376 | S2A_38SNG_20241108_0_L2A | 80 m | 37.1943, 45.8289 | 37.1655, 45.8549 |  |
| 377 | S2B_52KDA_20240314_0_L2A | 20 m window | -22.4053, 128.5960 | -22.4117, 128.6005 | Lake Mackay (Australia) (10 km) |
| 378 | S2B_28RFT_20240715_0_L2A | 20 m window | 28.9770, -13.8331 | 28.9811, -13.8140 | Timanfaya, Lanzarote Spain (8 km) |
| 379 | S2B_13RGK_20241008_0_L2A | 20 m window | 26.7581, -102.1063 | 26.8980, -102.1109 | Cuatro Cienegas gypsum dunes (Mexico) (3 km) |
| 380 | S2B_28WDT_20231030_0_L2A | 20 m window | 65.5892, -17.1401 | 65.5827, -17.1350 | Myvatn lava and lake, Iceland (7 km) |
| 381 | S2B_12VVJ_20231217_0_L2A | 20 m window | 56.9526, -111.3278 | 56.9494, -111.3323 | Athabasca oil sands mines, Alberta Canada (14 km) |
| 382 | S2B_38UPU_20240520_0_L2A | 20 m window | 48.3132, 47.0316 | 48.3025, 47.0115 | Lake Baskunchak, Russia (16 km) |
| 383 | S2A_37MCS_20230128_0_L2A | 20 m window | -3.0293, 37.4665 | -3.0947, 37.3917 | Kilimanjaro, Tanzania (9 km) |
| 384 | S2B_37RCK_20240602_0_L2A | 80 m | 26.3010, 37.4881 | 26.2874, 37.5007 |  |
| 385 | S2B_45TYE_20241023_0_L2A | 80 m | 39.8731, 90.5466 | 39.6783, 89.7762 |  |
| 386 | S2A_12UVB_20240318_0_L2A | 20 m window | 50.6539, -111.5866 | 50.6605, -111.5865 | Dinosaur Provincial Park badlands, Alberta Canada (12 km) |
| 387 | S2A_45TYH_20241001_0_L2A | 80 m | 43.2335, 90.4852 | 43.1130, 90.3396 |  |
| 388 | S2A_18LZH_20240528_0_L2A | 20 m window | -15.4814, -71.9912 | -15.4881, -71.9854 | Colca Canyon, Arequipa Peru (16 km) |
| 389 | S2B_12SVF_20240605_0_L2A | 80 m | 36.5530, -112.0865 | 36.7058, -112.0448 |  |
| 390 | S2B_20TMR_20241019_0_L2A | 80 m | 45.3607, -63.0902 | 45.3323, -63.0810 |  |
| 391 | S2B_36MXC_20240828_0_L2A | 20 m window | -2.5043, 34.7037 | -2.5537, 34.6519 | Serengeti plains, Tanzania (14 km) |
| 392 | S2A_45TYH_20241001_0_L2A | 80 m | 43.2229, 90.5115 | 43.2176, 90.4853 |  |
| 393 | S2B_11TQJ_20240830_0_L2A | 20 m window | 43.3460, -113.6132 | 43.3873, -113.6560 | Craters of the Moon lava field, Idaho USA (7 km) |
| 394 | S2A_46TCK_20240511_0_L2A | 80 m | 39.6705, 91.3691 | 39.7040, 91.3541 |  |
| 395 | S2B_40RFU_20240618_0_L2A | 20 m window | 30.3605, 59.1382 | 30.2573, 59.0851 | Dasht-e Lut kaluts, Iran (7 km) |
| 396 | S2A_34QHK_20240203_0_L2A | 80 m | 21.9325, 24.7991 | 21.9121, 24.7925 | Jebel Uweinat, Egypt/Libya/Sudan (16 km) |
| 397 | S2B_12SUE_20240615_0_L2A | 80 m | 35.6418, -112.1186 | 36.0438, -112.5587 |  |
| 398 | S2B_52KDA_20240314_0_L2A | 80 m | -22.6337, 128.7101 | -22.6493, 128.7774 | Lake Mackay, Australia (16 km) |
| 399 | S2A_48TUP_20241023_0_L2A | 80 m | 43.3353, 102.9717 | 43.2713, 102.9391 |  |
| 400 | S2A_47SNA_20240926_0_L2A | 80 m | 36.5087, 99.1247 | 36.5204, 99.1080 |  |
| 401 | S2A_48TUP_20241023_0_L2A | 80 m | 43.8883, 102.9243 | 43.9800, 102.9154 |  |
| 402 | S2A_46RBT_20240309_0_L2A | 20 m window | 28.8792, 90.6839 | 28.8841, 90.6803 | Yamdrok Tso, Tibet (8 km) |
| 403 | S2A_44SKA_20240930_0_L2A | 20 m window | 32.3648, 77.9486 | 32.3577, 77.9697 | Spiti Valley, India (15 km) |
| 404 | S2B_11SMA_20241009_0_L2A | 20 m window | 36.5343, -117.8708 | 36.5366, -117.8752 | Owens Lake (USA) (15 km) |
| 405 | S2B_34QBG_20241011_0_L2A | 20 m window | 19.8034, 18.5223 | 19.6438, 18.4423 | Emi Koussi, Tibesti Chad (10 km) |
| 406 | S2A_52JFS_20241221_0_L2A | 80 m | -25.8133, 130.4680 | -25.8452, 130.4484 |  |
| 407 | S2A_49KGP_20240107_0_L2A | 20 m window | -23.9176, 113.5388 | -23.9236, 113.5365 | Lake MacLeod, Carnarvon (14 km) |
| 408 | S2B_19KFP_20240407_0_L2A | 20 m window | -23.7128, -67.7450 | -23.7480, -67.7320 | Lagunas Miscanti and Minique, Chile (3 km) |
| 409 | S2B_44SKC_20240925_0_L2A | 20 m window | 33.8280, 78.6842 | 33.8215, 78.6910 | Pangong Tso, Ladakh (9 km) |
| 410 | S2B_51KVB_20240929_0_L2A | 80 m | -17.2262, 122.5203 | -17.2315, 122.5057 |  |
| 411 | S2A_37MCS_20230128_0_L2A | 20 m window | -3.0037, 37.3638 | -3.0127, 37.3522 | Kilimanjaro, Tanzania (7 km) |
| 412 | S2B_52KDF_20241030_0_L2A | 20 m window | -17.5048, 128.3159 | -17.5127, 128.3096 | Bungle Bungle (Purnululu), Australia (10 km) |
| 413 | S2A_53HKE_20240827_1_L2A | 80 m | -31.6717, 132.7026 | -31.6176, 132.1506 |  |
| 414 | S2B_37REJ_20240602_0_L2A | 20 m window | 25.8144, 39.8439 | 25.8132, 39.8515 | Harrat Khaybar, Saudi Arabia (5 km) |
| 415 | S2A_50SPG_20240425_0_L2A | 20 m window | 37.7808, 119.2364 | 37.7889, 119.2374 | Yellow River Delta, China (9 km) |
| 416 | S2B_54HXF_20241218_0_L2A | 80 m | -35.5755, 143.1987 | -35.5989, 143.1925 |  |
| 417 | S2A_29VPD_20230420_0_L2A | 80 m | 57.1917, -6.2869 | 57.2072, -6.2676 | Cuillin Hills, Isle of Skye, Scotland UK (4 km) |
| 418 | S2A_33SWB_20241015_0_L2A | 20 m window | 37.7063, 15.0117 | 37.7411, 15.0604 | Mount Etna, Sicily Italy (4 km) |
| 419 | S2A_43TEM_20240621_0_L2A | 80 m | 46.4879, 75.4802 | 46.4622, 75.4837 | Lake Balkhash, Kazakhstan (14 km) |
| 420 | S2B_13SDT_20240527_0_L2A | 20 m window | 33.7951, -105.9174 | 33.8008, -105.9124 | Carrizozo Malpais lava flow, New Mexico USA (11 km) |
| 421 | S2A_33KVQ_20240503_0_L2A | 20 m window | -22.8939, 14.4424 | -23.0133, 14.5072 | Walvis Bay salt works (Namibia) (6 km) |
| 422 | S2A_16XDP_20240706_1_L2A | 80 m | 79.9525, -87.0670 | 79.9893, -87.1319 |  |
| 423 | S2B_26WPE_20240717_0_L2A | 20 m window | 71.2173, -23.7088 | 71.2211, -23.7173 | Jameson Land, East Greenland (12 km) |
| 424 | S2A_27WWM_20240707_0_L2A | 80 m | 64.6498, -20.6980 | 64.6275, -20.6893 |  |
| 425 | S2A_47TQJ_20241006_1_L2A | 80 m | 43.6237, 102.0615 | 43.7123, 102.3766 | Khongoryn Els dunes, Mongolia (13 km) |
| 426 | S2A_30SWG_20240621_0_L2A | 80 m | 37.4205, -2.0693 | 37.4564, -2.1032 |  |
| 427 | S2B_12SVF_20241003_0_L2A | 20 m window | 36.7852, -111.9495 | 36.9033, -111.8743 | Vermilion Cliffs / Paria Plateau, Arizona USA (10 km) |
| 428 | S2B_28WDT_20231030_0_L2A | 80 m | 65.6985, -14.9114 | 65.6923, -14.8161 |  |
| 429 | S2B_28WDT_20231030_0_L2A | 80 m | 65.7636, -15.0453 | 65.7983, -15.1184 |  |
| 430 | S2A_59GMM_20230605_0_L2A | 20 m window | -44.1349, 170.1774 | -44.1243, 170.2129 | Lake Pukaki, New Zealand (10 km) |
| 431 | S2A_18FXK_20240516_0_L2A | 20 m window | -50.3845, -73.1116 | -50.3866, -73.1024 | Perito Moreno and Lago Argentino, Patagonia (10 km) |
| 432 | S2B_29VPE_20240919_0_L2A | 20 m window | 57.8763, -6.9171 | 57.8661, -6.9169 | Sound of Harris, Outer Hebrides (16 km) |
| 433 | S2A_27WWM_20240707_0_L2A | 20 m window | 63.9912, -18.9609 | 63.9906, -18.9684 | Landmannalaugar rhyolite, Iceland (5 km) |
| 434 | S2B_40RFU_20240621_0_L2A | 20 m window | 30.4998, 58.4082 | 30.5102, 58.3981 | Kaluts, Dasht-e Lut (Iran) (12 km) |
| 435 | S2B_40RFU_20240621_0_L2A | 80 m | 30.5701, 58.3064 | 30.5547, 58.3234 | Kaluts, Dasht-e Lut (Iran) (20 km) |
| 436 | S2A_19KEQ_20240103_0_L2A | 80 m | -23.0987, -68.4199 | -23.1050, -68.3970 | Valle de la Luna / Cordillera de la Sal, Chile (24 km) |
| 437 | S2A_19KFQ_20240919_0_L2A | 20 m window | -22.8041, -67.7357 | -22.8038, -67.7621 | Laguna Verde / Licancabur, Bolivia (8 km) |
| 438 | S2A_36MZC_20230128_0_L2A | 20 m window | -2.3844, 36.0943 | -2.3806, 36.0938 | Lake Natron, Tanzania (5 km) |
| 439 | S2B_46RBU_20241109_0_L2A | 80 m | 30.1307, 89.9452 | 30.1619, 89.9533 |  |
| 440 | S2B_20LNP_20240901_0_L2A | 80 m | -10.4764, -62.3935 | -10.4800, -62.4098 | Rondonia fishbone deforestation, Brazil (29 km) |
| 441 | S2B_20LNP_20240901_0_L2A | 80 m | -10.3039, -62.1429 | -10.4509, -62.2526 |  |
| 442 | S2A_36MYB_20230720_0_L2A | 80 m | -3.0616, 35.1598 | -3.1597, 35.0833 |  |
| 443 | S2B_38SNG_20240706_0_L2A | 80 m | 37.6377, 45.6087 | 37.6135, 45.6298 | Lake Urmia, Iran (15 km) |
| 444 | S2A_14SKG_20241006_0_L2A | 20 m window | 37.5292, -101.4264 | 37.5254, -101.4273 | Southwest Kansas centre pivots, USA (24 km) |
| 445 | S2A_33RYH_20240611_0_L2A | 20 m window | 24.9449, 17.8098 | 24.9725, 17.8530 | Waw an Namus, Libya (8 km) |
| 446 | S2B_38SNG_20240706_0_L2A | 20 m window | 37.3326, 45.4944 | 37.3201, 45.4984 | Lake Urmia, Iran (12 km) |
| 447 | S2B_39TXF_20240809_0_L2A | 20 m window | 41.3026, 53.3188 | 41.3843, 53.2118 | Kara-Bogaz-Gol, Turkmenistan (20 km) |
| 448 | S2B_12TTL_20240320_0_L2A | 20 m window | 40.8006, -113.9609 | 40.7946, -113.9519 | Bonneville Salt Flats, Utah (7 km) |
| 449 | S2B_13RGK_20241008_0_L2A | 20 m window | 26.7791, -102.0817 | 26.9063, -102.1351 | Cuatro Cienegas gypsum dunes (Mexico) (1 km) |
| 450 | S2B_38KNA_20240422_0_L2A | 20 m window | -22.5134, 45.3125 | -22.5057, 45.3074 | Isalo Massif, Madagascar (6 km) |
| 451 | S2B_12SUE_20240615_0_L2A | 80 m | 35.3814, -112.1392 | 35.6909, -112.4362 |  |
| 452 | S2B_46RBT_20241030_0_L2A | 20 m window | 28.9928, 90.5800 | 28.9948, 90.5867 | Yamdrok Tso, Tibet (12 km) |
| 453 | S2A_19JDM_20240323_0_L2A | 20 m window | -26.1386, -69.0439 | -26.1477, -69.0563 | Salar de Pedernales, Chile (13 km) |
| 454 | S2B_20LNP_20240901_0_L2A | 20 m window | -10.2883, -62.6769 | -10.2869, -62.6843 | Rondonia fishbone deforestation, Brazil (9 km) |
| 455 | S2A_50SPG_20240425_0_L2A | 80 m | 37.8204, 119.1493 | 37.8460, 119.2920 | Yellow River Delta, China (11 km) |
| 456 | S2A_50SPG_20240425_0_L2A | 20 m window | 37.7968, 119.2551 | 37.8094, 119.2819 | Yellow River Delta, China (12 km) |
| 457 | S2B_47TQJ_20240623_0_L2A | 20 m window | 43.6603, 102.1315 | 43.6952, 102.2582 | Khongoryn Els dunes, Mongolia (11 km) |
| 458 | S2B_33XWH_20230705_1_L2A | 20 m window | 78.5161, 16.8390 | 78.5088, 16.8573 | Billefjorden, Svalbard (12 km) |
| 459 | S2A_35KLT_20240122_0_L2A | 20 m window | -20.6495, 26.0138 | -20.6560, 25.9939 | Sua Pan, Makgadikgadi Botswana (12 km) |
| 460 | S2A_13RBL_20240522_0_L2A | 80 m | 27.7638, -107.7111 | 27.7859, -107.7195 |  |
| 461 | S2A_38SNG_20241108_0_L2A | 20 m window | 37.3916, 45.5573 | 37.3804, 45.5729 | Lake Urmia, Iran (3 km) |
| 462 | S2A_38LPQ_20240907_0_L2A | 20 m window | -9.4477, 46.4137 | -9.4462, 46.4103 | Aldabra Atoll, Seychelles (7 km) |
| 463 | S2A_28RCS_20240414_0_L2A | 20 m window | 28.3059, -16.6628 | 28.2929, -16.6630 | Teide caldera, Tenerife Spain (4 km) |
| 464 | S2A_16XDP_20240706_1_L2A | 20 m window | 79.4843, -90.6632 | 79.4736, -90.6779 | Axel Heiberg Island, Nunavut Canada (4 km) |
| 465 | S2B_44TLP_20241008_0_L2A | 80 m | 43.2724, 79.6097 | 43.4916, 79.5365 |  |
| 466 | S2A_50JKR_20241219_0_L2A | 80 m | -26.7051, 114.5021 | -26.6906, 114.4837 |  |
| 467 | S2B_50KPA_20240205_0_L2A | 80 m | -22.1664, 118.6685 | -22.1829, 118.6905 |  |
| 468 | S2B_52JFS_20240410_0_L2A | 20 m window | -25.3869, 130.8051 | -25.3983, 130.8197 | Uluru-Kata Tjuta, Australia (11 km) |
| 469 | S2B_52JFT_20240917_0_L2A | 80 m | -25.0588, 130.5619 | -24.9745, 130.6273 | Lake Amadeus, Northern Territory (21 km) |
| 470 | S2B_8WMB_20241004_0_L2A | 80 m | 68.7157, -136.6362 | 68.7482, -136.6743 |  |
| 471 | S2B_50KPA_20240205_0_L2A | 20 m window | -22.4452, 118.2887 | -22.4627, 118.3568 | Karijini, Hamersley Range Australia (8 km) |
| 472 | S2B_46SFF_20240818_0_L2A | 20 m window | 36.7386, 95.2282 | 36.7297, 95.2408 | Qarhan Salt Lake, Qinghai China (8 km) |
| 473 | S2B_28QDM_20241102_0_L2A | 20 m window | 23.9118, -15.8217 | 23.8617, -15.8629 | Dakhla lagoon, Western Sahara (10 km) |
| 474 | S2A_17SKV_20240329_0_L2A | 20 m window | 35.6040, -83.6481 | 35.6551, -83.6483 | Great Smoky Mountains, USA (14 km) |
| 475 | S2A_17SKV_20240329_0_L2A | 20 m window | 35.6008, -83.4595 | 35.5007, -83.5540 | Great Smoky Mountains, USA (6 km) |
| 476 | S2B_10TES_20241107_0_L2A | 20 m window | 46.1599, -122.2158 | 46.1272, -122.2300 | Mount St. Helens, Washington USA (7 km) |
| 477 | S2B_49KGR_20241018_0_L2A | 80 m | -22.3662, 113.4451 | -21.7672, 113.7679 |  |
| 478 | S2B_60HUB_20240509_0_L2A | 20 m window | -39.2141, 175.7135 | -39.2190, 175.7163 | Tongariro volcanic plateau, New Zealand (9 km) |
| 479 | S2B_58KDC_20240619_0_L2A | 20 m window | -20.2732, 164.2915 | -20.2798, 164.2901 | New Caledonia northern lagoon (10 km) |
| 480 | S2B_8WMB_20241004_0_L2A | 20 m window | 68.9794, -134.9667 | 68.9875, -134.9878 | Mackenzie River Delta, Canada (2 km) |
| 481 | S2B_60HUB_20240509_0_L2A | 80 m | -39.1889, 175.7988 | -39.3903, 175.7072 | Tongariro volcanic plateau, New Zealand (18 km) |
| 482 | S2B_18QZJ_20230319_0_L2A | 80 m | 21.0526, -71.6643 | 21.0914, -71.9046 |  |
| 483 | S2B_36KYB_20231106_0_L2A | 80 m | -20.8395, 35.3540 | -21.4523, 35.4521 |  |
| 484 | S2B_8WMB_20241004_0_L2A | 20 m window | 68.9279, -134.9710 | 68.9190, -135.0595 | Mackenzie River Delta, Canada (9 km) |
| 485 | S2A_36MZC_20230128_0_L2A | 20 m window | -2.4743, 36.0626 | -2.4709, 36.0306 | Lake Natron, Tanzania (8 km) |
| 486 | S2B_18GXP_20230619_0_L2A | 20 m window | -46.6298, -72.7158 | -46.6411, -72.6990 | Lago General Carrera, Chile (4 km) |
| 487 | S2B_37PFR_20240417_0_L2A | 20 m window | 13.5243, 40.7251 | 13.5310, 40.7552 | Erta Ale volcano, Ethiopia (11 km) |
| 488 | S2A_49KGR_20240206_0_L2A | 20 m window | -22.4475, 113.7164 | -22.4612, 113.7308 | Ningaloo Reef, Australia (18 km) |
| 489 | S2B_33XXJ_20240802_0_L2A | 80 m | 79.3215, 21.0631 | 79.1900, 22.6548 |  |
| 490 | S2B_18RTN_20230226_0_L2A | 80 m | 24.9316, -77.0850 | 24.8948, -77.0006 |  |
| 491 | S2B_18RTN_20230226_0_L2A | 80 m | 24.8585, -77.0545 | 24.9558, -77.0625 |  |
| 492 | S2A_5QKB_20230112_0_L2A | 80 m | 19.5986, -155.3222 | 19.6156, -155.2930 | Kilauea caldera and East Rift, Hawaii USA (24 km) |
| 493 | S2A_49KGR_20240206_0_L2A | 80 m | -22.1911, 113.3894 | -21.8329, 113.7319 |  |
| 494 | S2A_28RCS_20240414_0_L2A | 80 m | 28.3151, -16.0629 | 28.6284, -16.0541 |  |
| 495 | S2B_40RDQ_20240313_0_L2A | 80 m | 26.2712, 56.0494 | 26.4340, 56.2533 | Musandam Peninsula, Oman (23 km) |
| 496 | S2A_37PFT_20240522_0_L2A | 80 m | 15.8816, 40.9463 | 15.9291, 40.9079 |  |
| 497 | S2A_28WDS_20240614_0_L2A | 20 m window | 64.0077, -16.0110 | 64.0547, -16.1266 | Jokulsarlon and Breidamerkurjokull, Iceland (9 km) |
| 498 | S2A_35VNJ_20240905_0_L2A | 80 m | 61.7501, 28.2615 | 61.7326, 28.4235 | Saimaa Lakeland, Finland (23 km) |
| 499 | S2B_31UFV_20240921_0_L2A | 20 m window | 53.3060, 5.2786 | 53.2696, 5.2830 | Wadden Sea near Terschelling, Netherlands (8 km) |
| 500 | S2B_35SLA_20241008_0_L2A | 80 m | 36.9432, 25.7487 | 36.6713, 25.2276 |  |
| 501 | S2A_28RCS_20240414_0_L2A | 80 m | 28.5544, -16.0772 | 28.5825, -16.1516 |  |
| 502 | S2B_18FXJ_20240819_0_L2A | 20 m window | -51.0285, -72.9004 | -51.0299, -72.8937 | Torres del Paine, Chile (8 km) |
| 503 | S2B_57VXC_20241015_0_L2A | 20 m window | 56.1265, 161.0177 | 56.0951, 160.9825 | Klyuchevskaya volcano group, Kamchatka (23 km) |
| 504 | S2A_58KFA_20230408_0_L2A | 20 m window | -22.2940, 166.9860 | -22.2834, 166.9756 | Goro nickel laterite, New Caledonia (8 km) |
| 505 | S2B_22MEU_20240817_0_L2A | 20 m window | -6.1015, -50.1953 | -6.1018, -50.1925 | Carajas iron mine, Brazil (8 km) |
| 506 | S2B_21KVA_20240922_0_L2A | 20 m window | -17.8699, -57.2010 | -17.8947, -57.1726 | Paraguay River Pantanal wetlands, Brazil (9 km) |
| 507 | S2A_60HUB_20230907_0_L2A | 20 m window | -39.1902, 175.4917 | -39.1444, 175.5112 | Tongariro volcanic plateau, New Zealand (13 km) |
| 508 | S2A_18LZH_20240528_0_L2A | 20 m window | -15.5121, -72.0249 | -15.4828, -72.0147 | Colca Canyon, Arequipa Peru (17 km) |
| 509 | S2B_37PHN_20241130_0_L2A | 20 m window | 11.5594, 42.5303 | 11.5593, 42.4567 | Lake Assal, Djibouti (13 km) |
| 510 | S2A_51HWC_20240221_0_L2A | 80 m | -34.3496, 123.5262 | -34.3733, 123.8676 |  |
| 511 | S2A_49KGP_20240107_0_L2A | 80 m | -23.7092, 113.1579 | -23.6402, 113.6085 |  |
| 512 | S2B_8WMB_20241004_0_L2A | 20 m window | 69.0774, -134.8719 | 69.0708, -134.9099 | Mackenzie River Delta, Canada (9 km) |
| 513 | S2B_8WMB_20241004_0_L2A | 20 m window | 69.0679, -135.3639 | 69.0708, -135.3759 | Mackenzie River Delta, Canada (17 km) |
| 514 | S2B_54TXP_20240903_0_L2A | 20 m window | 43.5842, 142.7576 | 43.5864, 142.7711 | Daisetsuzan, Hokkaido, Japan (10 km) |
| 515 | S2B_32UME_20240501_0_L2A | 80 m | 54.0352, 8.0523 | 53.9556, 8.6372 |  |
| 516 | S2A_38SLH_20240806_0_L2A | 20 m window | 38.5806, 42.9456 | 38.6112, 42.9236 | Lake Van, Turkey (3 km) |
| 517 | S2A_50JKP_20241209_0_L2A | 80 m | -28.9558, 114.0931 | -28.4668, 114.4795 |  |
| 518 | S2B_35SLA_20241008_0_L2A | 80 m | 36.2526, 25.9020 | 36.8934, 25.0453 | Santorini caldera, Greece (20 km) |
| 519 | S2A_28RCS_20240414_0_L2A | 80 m | 28.5704, -16.0730 | 28.5834, -16.1515 |  |
| 520 | S2B_8WMB_20241004_0_L2A | 20 m window | 68.9799, -134.7829 | 68.9833, -134.7941 | Mackenzie River Delta, Canada (9 km) |
| 521 | S2B_8WMB_20241004_0_L2A | 20 m window | 69.1012, -135.0241 | 69.1076, -135.0111 | Mackenzie River Delta, Canada (12 km) |
| 522 | S2B_12SVG_20240605_0_L2A | 80 m | 37.8909, -111.8196 | 37.8969, -111.8473 |  |
| 523 | S2B_37PFR_20240417_0_L2A | 20 m window | 13.6760, 40.7131 | 13.7064, 40.7702 | Erta Ale volcano, Ethiopia (13 km) |
| 524 | S2A_49RDH_20240312_0_L2A | 20 m window | 24.9865, 110.4537 | 24.9653, 110.4381 | Yangshuo-Guilin karst, China (8 km) |
| 525 | S2B_11TQJ_20240402_0_L2A | 20 m window | 43.2299, -113.4540 | 43.3473, -113.5025 | Craters of the Moon lava field, Idaho USA (9 km) |
| 526 | S2B_18RTN_20230226_0_L2A | 20 m window | 24.5286, -77.8602 | 24.5331, -77.8719 | Andros and Tongue of the Ocean, Bahamas (8 km) |
| 527 | S2B_46SFF_20240818_0_L2A | 20 m window | 36.8832, 95.0871 | 36.9005, 95.1211 | Qarhan salt lake ponds, Qinghai (9 km) |
| 528 | S2B_19UES_20240829_0_L2A | 20 m window | 51.2514, -68.7433 | 51.2439, -68.7413 | Manicouagan Reservoir, Quebec, Canada (17 km) |
| 529 | S2B_20TMR_20241019_0_L2A | 20 m window | 45.2317, -64.1636 | 45.3503, -63.9865 | Minas Basin, Bay of Fundy (7 km) |
| 530 | S2B_8WMB_20241004_0_L2A | 20 m window | 68.8924, -135.1890 | 68.8938, -135.1728 | Mackenzie River Delta, Canada (14 km) |
| 531 | S2A_43TEM_20241128_0_L2A | 20 m window | 46.5044, 75.4423 | 46.5165, 75.4329 | Lake Balkhash, Kazakhstan (11 km) |

### Rock

| # | Scene | Image | Start | End | Near |
| ---: | --- | --- | --- | --- | --- |
| 0 | S2A_16RBT_20240511_0_L2A | 20 m window | 29.2159, -89.2294 | 29.2170, -89.2363 | Mississippi bird's-foot delta, USA (8 km) |
| 1 | S2A_22WEV_20240812_0_L2A | 20 m window | 67.1381, -50.1120 | 67.1779, -49.9585 | Greenland ice margin, Kangerlussuaq (4 km) |
| 2 | S2B_26WPE_20240717_0_L2A | 20 m window | 71.2686, -23.3814 | 71.2467, -23.3519 | Jameson Land, East Greenland (7 km) |
| 3 | S2B_44RNV_20241002_0_L2A | 20 m window | 30.9029, 81.4380 | 30.8071, 81.4373 | Mount Kailash / Lake Manasarovar, Tibet (1 km) |
| 4 | S2A_50JKP_20241209_0_L2A | 20 m window | -28.1221, 114.3002 | -28.1266, 114.3020 | Hutt Lagoon (Australia) (6 km) |
| 5 | S2B_12SVF_20241003_0_L2A | 80 m | 36.8196, -112.0454 | 36.7559, -112.0899 | Vermilion Cliffs / Paria Plateau, Arizona USA (25 km) |
| 6 | S2A_19KHQ_20240320_0_L2A | 20 m window | -23.0822, -65.4556 | -23.0848, -65.4523 | Quebrada de Humahuaca, Jujuy Argentina (17 km) |
| 7 | S2A_33RYH_20240611_0_L2A | 20 m window | 24.8758, 17.7072 | 24.8847, 17.6827 | Waw an Namus (Libya) (7 km) |
| 8 | S2A_20GMU_20241226_0_L2A | 20 m window | -42.4240, -63.9264 | -42.4073, -63.9062 | Península Valdés (Argentina) (12 km) |
| 9 | S2A_18FXK_20240516_0_L2A | 80 m | -50.4130, -72.2971 | -50.4149, -72.2675 |  |
| 10 | S2B_50RMT_20241106_0_L2A | 20 m window | 29.2084, 116.2475 | 29.2015, 116.2081 | Poyang Lake wetlands, China (12 km) |
| 11 | S2B_19JEG_20240315_0_L2A | 20 m window | -29.9964, -68.0539 | -29.9926, -68.0593 | Ischigualasto and Talampaya (Argentina) (15 km) |
| 12 | S2B_18FXJ_20240819_0_L2A | 80 m | -51.3755, -72.5487 | -51.3596, -72.5571 |  |
| 13 | S2B_50RMT_20241106_0_L2A | 20 m window | 29.1518, 116.1935 | 29.1390, 116.1894 | Poyang Lake wetlands, China (5 km) |
| 14 | S2A_44RNV_20240417_0_L2A | 80 m | 31.0440, 81.9830 | 31.0198, 81.9852 |  |
| 15 | S2A_37RCK_20241005_0_L2A | 80 m | 26.4141, 37.7356 | 26.4762, 37.7369 | AlUla sandstone (Saudi Arabia) (24 km) |
| 16 | S2A_50JKR_20241219_0_L2A | 80 m | -26.9095, 114.9406 | -26.9743, 115.0460 |  |
| 17 | S2A_5QKC_20241003_0_L2A | 80 m | 20.4568, -154.8568 | 20.4505, -154.8386 |  |
| 18 | S2A_43SFV_20240923_0_L2A | 80 m | 36.0631, 76.3779 | 36.0728, 76.3302 |  |
| 19 | S2A_36SXH_20241004_0_L2A | 80 m | 38.0941, 34.5122 | 38.0579, 34.4857 |  |
| 20 | S2A_19JFM_20240519_0_L2A | 80 m | -26.0011, -67.0415 | -25.9706, -67.0465 |  |
| 21 | S2B_43TFH_20230510_0_L2A | 80 m | 42.7321, 76.3540 | 42.5957, 76.4078 |  |
| 22 | S2A_12SUG_20240620_0_L2A | 80 m | 37.7130, -112.3965 | 37.6615, -112.3330 | Bryce Canyon, Utah USA (20 km) |
| 23 | S2A_32QMF_20241008_0_L2A | 80 m | 18.3413, 8.1144 | 18.3347, 8.1010 |  |
| 24 | S2A_35VNJ_20240905_0_L2A | 80 m | 61.7394, 28.5287 | 61.8053, 28.4581 | Saimaa Lakeland, Finland (22 km) |
| 25 | S2B_45TYE_20241023_0_L2A | 80 m | 40.0630, 90.3429 | 39.8354, 90.1642 |  |
| 26 | S2B_27VXL_20240629_0_L2A | 20 m window | 63.8490, -17.2331 | 63.7861, -17.0436 | Skeidararsandur outwash plain, Iceland (10 km) |
| 27 | S2A_14WPS_20240727_0_L2A | 80 m | 64.3011, -96.4979 | 64.3279, -96.3440 | Baker Lake tundra, Nunavut Canada (29 km) |
| 28 | S2A_5QKC_20241003_0_L2A | 20 m window | 19.8498, -155.3798 | 19.8430, -155.4067 | Mauna Kea summit, Hawaii USA (9 km) |
| 29 | S2A_12TUK_20240613_0_L2A | 80 m | 39.8011, -112.9749 | 39.7304, -112.8314 |  |
| 30 | S2A_46WEB_20240913_0_L2A | 20 m window | 69.1079, 93.9416 | 69.1100, 93.9554 | Putorana Plateau, Siberia Russia (12 km) |
| 31 | S2B_37PFR_20240417_0_L2A | 20 m window | 13.5176, 40.7411 | 13.5060, 40.7616 | Erta Ale volcano, Ethiopia (13 km) |
| 32 | S2A_12SWH_20241005_0_L2A | 80 m | 38.0916, -110.8022 | 38.1193, -110.8165 | Capitol Reef / Waterpocket Fold, Utah USA (30 km) |
| 33 | S2A_7VEG_20240216_0_L2A | 80 m | 59.7174, -140.5455 | 59.7270, -140.5806 | Malaspina Glacier, Alaska (22 km) |
| 34 | S2A_19JFM_20240519_0_L2A | 80 m | -26.2074, -67.7799 | -26.1712, -67.7864 |  |
| 35 | S2B_13SDT_20240527_0_L2A | 80 m | 33.6083, -105.7483 | 33.6557, -105.6681 | Carrizozo Malpais lava flow, New Mexico USA (24 km) |
| 36 | S2A_38UPV_20240925_0_L2A | 80 m | 49.0166, 47.5750 | 49.5512, 47.6405 |  |
| 37 | S2A_33RYH_20240611_0_L2A | 20 m window | 24.9286, 17.7995 | 24.9023, 17.7754 | Waw an Namus, Libya (2 km) |
| 38 | S2B_38UPU_20240907_0_L2A | 20 m window | 48.3003, 46.7243 | 48.3086, 46.8246 | Lake Baskunchak, Russia (14 km) |
| 39 | S2A_28WDT_20240919_0_L2A | 80 m | 65.5378, -16.1465 | 65.4996, -16.2761 |  |
| 40 | S2A_50JKR_20241219_0_L2A | 80 m | -27.1795, 114.4421 | -27.1834, 114.5145 |  |
| 41 | S2B_13SDT_20240904_0_L2A | 20 m window | 33.7253, -105.9116 | 33.7368, -105.8803 | Carrizozo Malpais lava flow, New Mexico USA (6 km) |
| 42 | S2A_49MGM_20230624_0_L2A | 80 m | -7.6920, 112.8641 | -8.0636, 113.6417 |  |
| 43 | S2A_38SLH_20240806_0_L2A | 80 m | 38.3106, 42.9596 | 38.5353, 42.8113 | Lake Van, Turkey (20 km) |
| 44 | S2A_15TUG_20240523_0_L2A | 20 m window | 42.3369, -94.1496 | 42.3280, -94.1583 | Iowa corn belt farmland, USA (13 km) |
| 45 | S2B_44SKC_20240925_0_L2A | 80 m | 33.6314, 78.4547 | 33.6543, 78.4538 | Pangong Tso, Ladakh (22 km) |
| 46 | S2B_12SVE_20241003_0_L2A | 20 m window | 35.9873, -112.0446 | 35.9979, -112.0686 | Grand Canyon, Arizona USA (13 km) |
| 47 | S2A_59GMM_20230605_0_L2A | 80 m | -44.1404, 169.9171 | -44.1382, 169.8945 | Lake Pukaki, New Zealand (22 km) |
| 48 | S2B_12VVJ_20230829_0_L2A | 20 m window | 56.9629, -111.3513 | 56.9682, -111.3422 | Athabasca oil sands mines, Alberta Canada (13 km) |
| 49 | S2B_41XMD_20240731_0_L2A | 20 m window | 75.4245, 61.1610 | 75.4182, 61.1463 | Novaya Zemlya ice cap (10 km) |
| 50 | S2A_43TFH_20231012_0_L2A | 80 m | 43.2824, 77.0230 | 43.2641, 77.0290 |  |
| 51 | S2A_20GMU_20240420_0_L2A | 20 m window | -42.5258, -63.9905 | -42.5174, -63.9782 | Península Valdés (Argentina) (3 km) |
| 52 | S2B_12VVJ_20230829_0_L2A | 20 m window | 57.0515, -111.4717 | 57.0867, -111.5556 | Athabasca oil sands mines, Alberta Canada (8 km) |
| 53 | S2B_37PHN_20241130_0_L2A | 80 m | 11.5279, 42.4633 | 11.5090, 42.4532 | Lake Assal, Djibouti (15 km) |
| 54 | S2A_36MZC_20230128_0_L2A | 80 m | -2.1899, 36.0569 | -2.2825, 36.0987 | Lake Natron (Tanzania) (15 km) |
| 55 | S2A_4WEV_20240831_1_L2A | 20 m window | 67.0844, -158.8609 | 67.0820, -158.8546 | Great Kobuk Sand Dunes (USA) (5 km) |
| 56 | S2B_36MZD_20240805_0_L2A | 20 m window | -1.7049, 36.2813 | -1.7149, 36.2859 | Lake Magadi, Kenya (18 km) |
| 57 | S2B_49MHM_20231027_0_L2A | 80 m | -7.5365, 114.1318 | -7.3604, 113.9439 |  |
| 58 | S2B_26WPE_20240717_0_L2A | 80 m | 72.0311, -22.9983 | 72.0318, -22.9660 |  |
| 59 | S2B_19LDC_20240808_0_L2A | 80 m | -16.1292, -69.6195 | -16.1300, -69.6428 |  |
| 60 | S2B_13TGG_20240606_0_L2A | 20 m window | 42.2151, -101.2590 | 42.1370, -101.2925 | Nebraska Sand Hills, USA (13 km) |
| 61 | S2B_38SNG_20240706_0_L2A | 20 m window | 37.3552, 45.5940 | 37.3942, 45.6128 | Lake Urmia, Iran (3 km) |
| 62 | S2B_52KDA_20240314_0_L2A | 20 m window | -22.5324, 128.6586 | -22.5621, 128.7797 | Lake Mackay, Australia (6 km) |
| 63 | S2B_37REJ_20240602_0_L2A | 20 m window | 25.7515, 39.8525 | 25.7486, 39.8562 | Harrat Khaybar, Saudi Arabia (7 km) |
| 64 | S2B_19KFP_20240407_0_L2A | 20 m window | -23.7784, -67.7010 | -23.7205, -67.7357 | Lagunas Miscanti and Minique, Chile (5 km) |
| 65 | S2A_47SNA_20240330_0_L2A | 20 m window | 36.8732, 100.1113 | 36.8678, 100.1130 | Qinghai Lake, China (8 km) |
| 66 | S2B_30SXH_20240723_0_L2A | 20 m window | 37.9763, -0.8309 | 37.9911, -0.8293 | Laguna de Torrevieja, Spain (12 km) |
| 67 | S2B_35QKE_20240816_0_L2A | 20 m window | 21.8719, 24.9429 | 21.8737, 24.9770 | Jebel Uweinat, Egypt/Libya/Sudan (3 km) |
| 68 | S2A_57VXC_20240423_0_L2A | 20 m window | 55.9587, 160.6336 | 55.9635, 160.6333 | Klyuchevskaya volcano group, Kamchatka (10 km) |
| 69 | S2A_12SXD_20241005_0_L2A | 80 m | 34.7756, -108.8187 | 34.6632, -108.8837 |  |
| 70 | S2A_32SMC_20240207_0_L2A | 20 m window | 33.6787, 8.3063 | 33.6581, 8.3345 | Chott el Djerid (Tunisia) (8 km) |
| 71 | S2B_12VVJ_20230829_0_L2A | 80 m | 57.6716, -111.4837 | 57.4412, -112.6552 |  |
| 72 | S2A_27VXL_20241012_0_L2A | 20 m window | 63.6767, -18.0145 | 63.7041, -18.0907 | Eldhraun moss-covered lava, Iceland (10 km) |
| 73 | S2A_32VMP_20240919_0_L2A | 80 m | 61.4871, 7.3509 | 61.4969, 7.3090 | Jostedalsbreen and inner Sognefjord, Norway (21 km) |
| 74 | S2B_19KEQ_20240914_0_L2A | 20 m window | -22.9004, -68.1769 | -22.8742, -68.1679 | Valle de la Luna / Cordillera de la Sal, Chile (12 km) |
| 75 | S2A_33JYJ_20241203_0_L2A | 20 m window | -28.2624, 17.1221 | -28.2531, 17.1080 | Richtersveld, South Africa (14 km) |
| 76 | S2B_11TQJ_20240402_0_L2A | 80 m | 43.3785, -113.5311 | 43.3676, -113.4890 | Craters of the Moon lava field, Idaho USA (4 km) |
| 77 | S2B_11SPS_20240229_0_L2A | 80 m | 33.3518, -115.2828 | 33.3281, -115.2809 |  |
| 78 | S2A_37QCD_20241005_0_L2A | 80 m | 21.1872, 37.1678 | 21.1860, 37.2051 | Dungonab Bay, Sudan (10 km) |
| 79 | S2B_27PZS_20240506_0_L2A | 80 m | 14.5917, -17.4550 | 14.7084, -17.6286 |  |
| 80 | S2A_12SWH_20241005_0_L2A | 20 m window | 38.2183, -109.9715 | 38.2117, -109.9765 | Canyonlands (Island in the Sky/Needles), Utah USA (11 km) |
| 81 | S2A_30SWG_20240621_0_L2A | 80 m | 36.9768, -2.4866 | 37.0034, -2.3340 | Tabernas Desert, Almeria Spain (7 km) |
| 82 | S2B_48PXR_20230313_0_L2A | 20 m window | 9.6464, 106.6165 | 9.6914, 106.6116 | Mekong River mouths, Vietnam (15 km) |
| 83 | S2A_37REJ_20240826_0_L2A | 80 m | 25.4260, 39.1883 | 25.4767, 39.1348 |  |
| 84 | S2A_20KKU_20240718_0_L2A | 80 m | -23.8646, -65.3833 | -23.8450, -65.3700 |  |
| 85 | S2B_60HUB_20240509_0_L2A | 20 m window | -39.1237, 175.6658 | -39.1165, 175.6365 | Tongariro volcanic plateau, New Zealand (3 km) |
| 86 | S2B_11SNA_20240611_0_L2A | 80 m | 36.3568, -115.8669 | 36.4990, -116.1683 |  |
| 87 | S2A_12SVH_20240620_0_L2A | 20 m window | 38.2043, -111.2647 | 38.2103, -111.2695 | Capitol Reef / Waterpocket Fold, Utah USA (16 km) |
| 88 | S2A_19JFG_20240419_0_L2A | 20 m window | -29.9607, -67.8772 | -29.9810, -67.8790 | Ischigualasto and Talampaya (Argentina) (4 km) |
| 89 | S2B_36KYB_20231106_0_L2A | 80 m | -21.3937, 35.4994 | -21.3228, 35.5470 |  |
| 90 | S2A_16QDF_20240205_0_L2A | 20 m window | 18.5026, -87.3258 | 18.5037, -87.3310 | Banco Chinchorro, Mexico (11 km) |
| 91 | S2B_52KDA_20240314_0_L2A | 20 m window | -22.4638, 128.8202 | -22.4577, 128.8173 | Lake Mackay, Australia (13 km) |
| 92 | S2A_54JTL_20241219_0_L2A | 20 m window | -31.5600, 138.5137 | -31.5599, 138.5191 | Flinders Ranges (Wilpena Pound), Australia (8 km) |
| 93 | S2B_38PKT_20240603_0_L2A | 20 m window | 11.5155, 42.3620 | 11.5143, 42.3530 | Lake Assal, Djibouti (16 km) |
| 94 | S2A_43RDP_20240615_0_L2A | 80 m | 30.2292, 74.1484 | 30.2725, 74.1497 |  |
| 95 | S2B_38KNA_20240422_0_L2A | 80 m | -21.7971, 45.4564 | -21.8017, 45.4799 |  |
| 96 | S2A_17SKV_20240329_0_L2A | 80 m | 35.5518, -83.1574 | 35.4524, -83.1672 |  |
| 97 | S2B_11TQJ_20240830_0_L2A | 20 m window | 43.2945, -113.6412 | 43.2906, -113.6425 | Craters of the Moon lava field, Idaho USA (10 km) |
| 98 | S2A_37PFR_20240909_0_L2A | 20 m window | 13.6012, 40.6762 | 13.7031, 40.6505 | Erta Ale volcano, Ethiopia (6 km) |
| 99 | S2A_38LPQ_20240907_0_L2A | 20 m window | -9.3464, 46.2720 | -9.3460, 46.2926 | Aldabra Atoll, Seychelles (11 km) |
| 100 | S2B_44TLP_20241008_0_L2A | 80 m | 43.4879, 78.8054 | 43.4155, 78.5518 |  |
| 101 | S2A_37PFR_20240909_0_L2A | 80 m | 13.7439, 40.0757 | 13.7652, 40.0916 |  |
| 102 | S2A_53KKP_20240412_0_L2A | 80 m | -23.7096, 132.1319 | -23.8758, 132.0944 |  |
| 103 | S2B_19JEG_20240315_0_L2A | 80 m | -30.2093, -68.4806 | -30.3197, -68.5238 |  |
| 104 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.9474, 17.8612 | 24.9396, 17.8887 | Waw an Namus, Libya (11 km) |
| 105 | S2A_30SWG_20240621_0_L2A | 80 m | 37.7420, -2.1176 | 37.7245, -2.0721 |  |
| 106 | S2A_46WEB_20240913_0_L2A | 80 m | 68.7356, 93.3933 | 68.7373, 93.5701 |  |
| 107 | S2B_11TQJ_20240402_0_L2A | 20 m window | 43.3135, -113.5439 | 43.3108, -113.5311 | Craters of the Moon lava field, Idaho USA (4 km) |
| 108 | S2B_22KGU_20240114_0_L2A | 20 m window | -24.4203, -48.2366 | -24.4111, -48.2368 | Serra do Mar Atlantic Forest, Brazil (14 km) |
| 109 | S2A_37REJ_20241005_0_L2A | 20 m window | 25.9116, 39.7886 | 25.9176, 39.7811 | Harrat Khaybar, Saudi Arabia (17 km) |
| 110 | S2B_33KVS_20241114_0_L2A | 80 m | -21.3524, 14.4388 | -21.1462, 14.2325 | Brandberg massif, Namibia (25 km) |
| 111 | S2B_27VXL_20240629_0_L2A | 80 m | 63.8365, -17.3466 | 63.8814, -17.1000 | Skeidararsandur outwash plain, Iceland (5 km) |
| 112 | S2A_35TPL_20240705_0_L2A | 20 m window | 45.0884, 29.3817 | 45.0836, 29.3786 | Danube Delta, Romania (2 km) |
| 113 | S2B_10TEN_20240610_0_L2A | 20 m window | 42.9608, -122.1077 | 42.9466, -122.0975 | Crater Lake, Oregon, USA (2 km) |
| 114 | S2B_12SUE_20240615_0_L2A | 80 m | 35.8659, -112.6980 | 35.9155, -112.1767 |  |
| 115 | S2A_36QVF_20240305_0_L2A | 80 m | 18.0983, 32.6313 | 18.1103, 32.7951 | Bayuda volcanic field, Sudan (22 km) |
| 116 | S2A_30SWG_20240202_0_L2A | 20 m window | 37.0921, -2.3927 | 37.0873, -2.3937 | Tabernas Desert, Almeria Spain (4 km) |
| 117 | S2A_48UXD_20230512_0_L2A | 80 m | 52.4183, 106.6471 | 52.4334, 106.6748 |  |
| 118 | S2B_51XXA_20240710_0_L2A | 80 m | 72.0639, 126.6043 | 72.0840, 126.5545 |  |
| 119 | S2B_30PUB_20240501_0_L2A | 20 m window | 15.2659, -4.2953 | 15.2763, -4.2706 | Inner Niger Delta near Lake Debo, Mali (12 km) |
| 120 | S2B_42QWM_20241001_0_L2A | 20 m window | 23.9440, 70.0155 | 23.9197, 69.9865 | Rann of Kutch, India (4 km) |
| 121 | S2B_21KVA_20240922_0_L2A | 80 m | -17.3063, -57.8592 | -17.2560, -57.8576 |  |
| 122 | S2A_42UUA_20240921_0_L2A | 20 m window | 50.4138, 66.9145 | 50.3974, 66.8560 | Turgai steppe, Kazakhstan (13 km) |
| 123 | S2B_28WDT_20231030_0_L2A | 20 m window | 65.6173, -16.7583 | 65.6972, -17.0056 | Myvatn lava and lake, Iceland (8 km) |
| 124 | S2B_18FXJ_20240819_0_L2A | 80 m | -50.9850, -72.6701 | -50.8957, -72.6459 | Torres del Paine, Chile (25 km) |
| 125 | S2A_59GMM_20230605_0_L2A | 20 m window | -44.0454, 170.1087 | -44.0353, 170.1484 | Lake Pukaki, New Zealand (2 km) |
| 126 | S2B_5VLE_20240615_0_L2A | 80 m | 58.2103, -155.5121 | 58.1940, -155.4841 | Valley of Ten Thousand Smokes, Alaska USA (20 km) |
| 127 | S2A_54JTL_20241219_0_L2A | 20 m window | -31.6172, 138.7390 | -31.6265, 138.6987 | Flinders Ranges (Wilpena Pound), Australia (14 km) |
| 128 | S2A_29SQB_20240614_0_L2A | 80 m | 37.9002, -5.7931 | 37.1600, -5.9967 |  |
| 129 | S2B_26WNE_20240707_0_L2A | 20 m window | 71.4346, -25.3267 | 71.5047, -25.4446 | Scoresby Sund inner fjords, Greenland (5 km) |
| 130 | S2B_22KGU_20240114_0_L2A | 20 m window | -24.2409, -48.2383 | -24.2246, -48.2480 | Serra do Mar Atlantic Forest, Brazil (9 km) |
| 131 | S2A_30SWG_20240202_0_L2A | 20 m window | 37.0904, -2.5023 | 37.0052, -2.4028 | Tabernas Desert, Almeria Spain (5 km) |
| 132 | S2A_34KFD_20240214_0_L2A | 80 m | -19.7970, 22.4576 | -19.6042, 22.4924 |  |
| 133 | S2B_33JYJ_20240402_0_L2A | 80 m | -28.5630, 17.4330 | -28.2204, 17.7891 |  |
| 134 | S2A_47SND_20240926_0_L2A | 20 m window | 38.9870, 100.1183 | 39.0062, 100.1671 | Zhangye Danxia, Gansu China (6 km) |
| 135 | S2B_26WNE_20240707_0_L2A | 20 m window | 71.6002, -25.7762 | 71.5976, -25.7642 | Scoresby Sund inner fjords, Greenland (15 km) |
| 136 | S2B_37REJ_20240602_0_L2A | 80 m | 25.9905, 39.6465 | 26.0882, 39.6852 |  |
| 137 | S2A_30SWG_20240202_0_L2A | 20 m window | 37.0392, -2.4160 | 36.9854, -2.4772 | Tabernas Desert, Almeria Spain (6 km) |
| 138 | S2A_12UVB_20240318_0_L2A | 20 m window | 50.8185, -111.6066 | 50.8332, -111.5779 | Dinosaur Provincial Park badlands, Alberta Canada (11 km) |
| 139 | S2C_53KKP_20241108_1_L2A | 20 m window | -23.8143, 132.7665 | -23.6187, 132.7482 | MacDonnell Ranges, Australia (6 km) |
| 140 | S2A_45TTF_20240210_0_L2A | 20 m window | 41.0968, 84.6458 | 41.0870, 84.6154 | Tarim River poplar forest, China (15 km) |
| 141 | S2A_12TUK_20240613_0_L2A | 80 m | 40.1825, -113.0314 | 40.0155, -112.7562 |  |
| 142 | S2B_11SNA_20241009_0_L2A | 80 m | 36.4306, -116.4815 | 36.7344, -116.2603 |  |
| 143 | S2B_36RYV_20240310_0_L2A | 20 m window | 31.0970, 35.5332 | 31.0964, 35.5247 | Dead Sea evaporation ponds (8 km) |
| 144 | S2B_52KDF_20241030_0_L2A | 20 m window | -17.3868, 128.4345 | -17.3971, 128.4315 | Purnululu Bungle Bungles (Australia) (7 km) |
| 145 | S2B_50RMT_20241106_0_L2A | 80 m | 29.3750, 116.8455 | 29.4063, 116.9226 |  |
| 146 | S2B_44TLP_20241008_0_L2A | 20 m window | 43.9761, 78.7090 | 43.9865, 78.7173 | Aktau Mountains, Altyn-Emel Kazakhstan (13 km) |
| 147 | S2A_30VUH_20230420_0_L2A | 20 m window | 56.6522, -4.6459 | 56.6111, -4.7221 | Rannoch Moor, Scotland, UK (5 km) |
| 148 | S2A_32VMP_20240919_0_L2A | 80 m | 62.1524, 8.3733 | 62.0521, 7.7707 |  |
| 149 | S2A_29VPD_20230420_0_L2A | 20 m window | 57.2592, -6.3048 | 57.2799, -6.2244 | Cuillin Hills, Isle of Skye, Scotland UK (6 km) |
| 150 | S2A_28WDT_20230531_0_L2A | 80 m | 65.1674, -16.5695 | 65.1416, -16.5964 | Askja and Holuhraun lava field, Iceland (19 km) |
| 151 | S2B_52KDA_20240920_0_L2A | 80 m | -22.6156, 128.8541 | -22.6094, 128.9473 | Lake Mackay, Australia (24 km) |
| 152 | S2A_13RBL_20240929_0_L2A | 20 m window | 27.5062, -107.9225 | 27.4564, -107.9146 | Copper Canyon, Chihuahua Mexico (12 km) |
| 153 | S2A_46RBU_20240309_0_L2A | 20 m window | 30.6635, 90.4614 | 30.6449, 90.5373 | Nam Co, Tibet (11 km) |
| 154 | S2B_23LLG_20240930_0_L2A | 80 m | -12.4562, -46.4216 | -12.6474, -46.8385 |  |
| 155 | S2A_59GMM_20230605_0_L2A | 20 m window | -44.1540, 170.0547 | -44.1014, 170.0547 | Lake Pukaki, New Zealand (12 km) |
| 156 | S2B_21KVA_20240922_0_L2A | 80 m | -18.1066, -57.6330 | -17.9401, -57.8881 |  |
| 157 | S2B_33KXV_20241124_0_L2A | 80 m | -18.3402, 16.9483 | -18.4441, 16.5153 |  |
| 158 | S2B_19JEG_20240315_0_L2A | 20 m window | -29.9378, -67.9369 | -29.9801, -68.0122 | Ischigualasto and Talampaya (Argentina) (9 km) |
| 159 | S2A_35KLT_20240730_0_L2A | 80 m | -20.1820, 25.2078 | -20.0156, 25.3654 |  |
| 160 | S2B_11SMA_20241009_0_L2A | 80 m | 36.4981, -117.7395 | 36.3403, -117.6577 | Owens Lake, California (21 km) |
| 161 | S2A_36MZC_20230128_0_L2A | 20 m window | -2.4112, 36.0520 | -2.3952, 36.0035 | Lake Natron, Tanzania (0 km) |
| 162 | S2A_31TGJ_20241229_0_L2A | 20 m window | 43.9018, 5.9314 | 43.9606, 6.1332 | Valensole Plateau, Provence (9 km) |
| 163 | S2B_32SMC_20240731_0_L2A | 80 m | 33.4910, 8.9971 | 33.9290, 8.6615 |  |
| 164 | S2A_35KLT_20240122_0_L2A | 80 m | -20.3099, 25.9923 | -20.1486, 26.0660 |  |
| 165 | S2A_59GMM_20240220_0_L2A | 20 m window | -43.7006, 170.2178 | -43.7112, 170.2086 | Aoraki / Mount Cook, New Zealand (13 km) |
| 166 | S2A_18FXJ_20231208_0_L2A | 20 m window | -51.1106, -73.0469 | -51.1147, -72.9718 | Torres del Paine, Chile (7 km) |
| 167 | S2A_46WEB_20240913_0_L2A | 20 m window | 68.9570, 94.1100 | 68.9522, 94.1287 | Putorana Plateau, Siberia Russia (7 km) |
| 168 | S2A_32VMP_20240919_0_L2A | 20 m window | 61.7153, 8.5910 | 61.7117, 8.6138 | Jotunheimen, Norway (17 km) |
| 169 | S2A_32TQS_20230910_1_L2A | 20 m window | 46.5219, 11.7181 | 46.5343, 11.7020 | Dolomites, Italy (11 km) |
| 170 | S2B_18RTN_20230226_0_L2A | 20 m window | 24.6152, -77.8157 | 24.6120, -77.7910 | Andros and Tongue of the Ocean, Bahamas (5 km) |
| 171 | S2A_28RCS_20240414_0_L2A | 80 m | 28.2517, -16.7908 | 28.1448, -16.6256 | Teide caldera, Tenerife Spain (10 km) |
| 172 | S2A_16XDP_20240706_1_L2A | 80 m | 79.2187, -87.5314 | 79.4133, -88.6179 |  |
| 173 | S2B_11TQJ_20240830_0_L2A | 20 m window | 43.2932, -113.6555 | 43.3607, -113.7080 | Craters of the Moon lava field, Idaho USA (11 km) |
| 174 | S2A_30VUH_20230420_0_L2A | 80 m | 56.7581, -5.4159 | 56.8044, -5.0785 |  |
| 175 | S2B_57VXC_20241015_0_L2A | 80 m | 56.5057, 160.6516 | 56.3208, 160.7198 |  |
| 176 | S2B_37PFR_20240417_0_L2A | 20 m window | 13.6946, 40.7219 | 13.6107, 40.7371 | Erta Ale volcano, Ethiopia (9 km) |
| 177 | S2A_37PFR_20240909_0_L2A | 20 m window | 13.5822, 40.7717 | 13.6261, 40.6997 | Erta Ale volcano, Ethiopia (7 km) |
| 178 | S2B_26WPE_20240717_0_L2A | 20 m window | 71.2357, -23.3096 | 71.2187, -23.2112 | Jameson Land, East Greenland (12 km) |
| 179 | S2B_30PVA_20241107_0_L2A | 20 m window | 14.3659, -3.5784 | 14.3307, -3.6042 | Bandiagara Escarpment, Mali (10 km) |
| 180 | S2B_57VXC_20241015_0_L2A | 80 m | 56.5040, 161.7251 | 56.4250, 161.7674 |  |
| 181 | S2A_19JFG_20240419_0_L2A | 80 m | -30.1524, -67.1940 | -30.2915, -67.6325 |  |
| 182 | S2B_12SUG_20241003_0_L2A | 20 m window | 37.3434, -113.0473 | 37.3124, -113.1313 | Zion Canyon, Utah USA (12 km) |
| 183 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.8278, 17.7626 | 24.8634, 17.7840 | Waw an Namus, Libya (8 km) |
| 184 | S2B_32TMS_20241231_0_L2A | 80 m | 46.0868, 8.5553 | 46.0163, 8.2390 |  |
| 185 | S2B_12SUG_20241003_0_L2A | 20 m window | 37.1894, -113.0769 | 37.2964, -113.0605 | Zion Canyon, Utah USA (6 km) |
| 186 | S2B_34KFD_20241026_0_L2A | 20 m window | -19.2835, 22.9627 | -19.4048, 22.7439 | Okavango Delta, Botswana (7 km) |
| 187 | S2B_12SVE_20241003_0_L2A | 80 m | 36.1122, -111.8151 | 35.3367, -111.1857 |  |
| 188 | S2B_34KFD_20241026_0_L2A | 20 m window | -19.2572, 22.8926 | -19.4057, 22.7803 | Okavango Delta, Botswana (8 km) |
| 189 | S2A_18LZH_20240528_0_L2A | 80 m | -16.2150, -72.1824 | -15.9862, -72.0233 |  |
| 190 | S2B_11SLC_20241002_0_L2A | 20 m window | 37.9957, -119.1546 | 37.9600, -119.1383 | Mono Lake, California (13 km) |
| 191 | S2B_30PUB_20241107_0_L2A | 20 m window | 15.1713, -4.2936 | 15.2709, -4.2643 | Inner Niger Delta near Lake Debo, Mali (9 km) |
| 192 | S2B_60HUB_20240509_0_L2A | 20 m window | -39.1756, 175.5457 | -39.0802, 175.4990 | Tongariro volcanic plateau, New Zealand (11 km) |
| 193 | S2A_60HUB_20230907_0_L2A | 20 m window | -39.1558, 175.4949 | -39.2093, 175.5042 | Tongariro volcanic plateau, New Zealand (13 km) |
| 194 | S2A_31TGJ_20241229_0_L2A | 80 m | 43.8545, 6.0115 | 43.5537, 6.1033 | Valensole Plateau, Provence (17 km) |
| 195 | S2A_19LBE_20240528_0_L2A | 20 m window | -13.9632, -71.2953 | -13.9280, -71.1972 | Vinicunca (Rainbow Mountain), Cusco Peru (10 km) |
| 196 | S2B_26WPE_20240717_0_L2A | 20 m window | 71.2916, -23.4048 | 71.2994, -23.4041 | Jameson Land, East Greenland (3 km) |
| 197 | S2B_26WPE_20240717_0_L2A | 20 m window | 71.2364, -23.4382 | 71.2252, -23.4479 | Jameson Land, East Greenland (8 km) |
| 198 | S2A_33XWG_20240718_2_L2A | 20 m window | 78.2157, 17.3313 | 78.2515, 17.3153 | Nordenskiold Land, Svalbard Norway (14 km) |
| 199 | S2A_32TQS_20230910_1_L2A | 20 m window | 46.4228, 11.7251 | 46.3942, 11.7145 | Dolomites, Italy (14 km) |
| 200 | S2B_47UNS_20240818_0_L2A | 80 m | 50.7236, 99.9787 | 50.6006, 99.5026 |  |
| 201 | S2B_51XXA_20240710_0_L2A | 20 m window | 72.8941, 126.7473 | 72.8944, 126.7637 | Lena Delta, Russia (13 km) |
| 202 | S2A_31TGJ_20241229_0_L2A | 20 m window | 43.7880, 5.9917 | 43.8120, 6.0306 | Valensole Plateau, Provence (6 km) |
| 203 | S2A_28RCS_20240414_0_L2A | 20 m window | 28.1894, -16.5463 | 28.1892, -16.5519 | Teide caldera, Tenerife Spain (13 km) |
| 204 | S2A_39TUL_20240701_0_L2A | 80 m | 45.7533, 49.3330 | 45.8899, 49.3223 |  |
| 205 | S2B_26WPE_20240717_0_L2A | 20 m window | 71.2068, -23.8696 | 71.2152, -23.8343 | Jameson Land, East Greenland (16 km) |
| 206 | S2A_33XWG_20240718_2_L2A | 80 m | 78.1655, 17.4539 | 78.1491, 17.4882 | Nordenskiold Land, Svalbard Norway (22 km) |
| 207 | S2B_26WNE_20240707_0_L2A | 20 m window | 71.4698, -25.2604 | 71.4653, -25.3270 | Scoresby Sund inner fjords, Greenland (8 km) |
| 208 | S2A_4WEV_20240831_1_L2A | 80 m | 67.4318, -156.5848 | 67.4578, -156.6060 |  |
| 209 | S2A_18WXD_20240818_0_L2A | 80 m | 70.2557, -71.6835 | 70.2001, -71.0391 |  |
| 210 | S2A_44SKA_20240930_0_L2A | 80 m | 31.6633, 78.2445 | 31.7023, 78.1500 |  |
| 211 | S2B_44SKC_20240508_0_L2A | 80 m | 33.4586, 78.1055 | 33.4387, 78.1082 |  |
| 212 | S2A_36QVF_20240305_0_L2A | 20 m window | 18.3723, 32.8032 | 18.3832, 32.7517 | Bayuda volcanic field, Sudan (9 km) |
| 213 | S2B_12VVJ_20231217_0_L2A | 80 m | 57.4736, -112.0834 | 57.4916, -112.0749 |  |
| 214 | S2A_22WEB_20240716_0_L2A | 20 m window | 69.1380, -50.1164 | 69.1240, -50.2270 | Ilulissat Icefjord, Greenland (6 km) |
| 215 | S2B_57VXC_20241015_0_L2A | 20 m window | 56.0388, 160.9054 | 56.0158, 161.0271 | Klyuchevskaya volcano group, Kamchatka (20 km) |
| 216 | S2A_43TDM_20241009_0_L2A | 80 m | 46.8888, 74.7825 | 46.8906, 74.9679 |  |
| 217 | S2A_36NZJ_20240704_0_L2A | 80 m | 2.7819, 35.9529 | 2.8504, 35.9671 |  |
| 218 | S2B_37PDQ_20240301_0_L2A | 20 m window | 13.1272, 38.1691 | 13.1439, 38.1853 | Simien Mountains, Ethiopia (13 km) |
| 219 | S2B_20TMR_20241019_0_L2A | 80 m | 45.4711, -63.0665 | 45.4443, -63.0361 |  |
| 220 | S2B_11TQJ_20240830_0_L2A | 80 m | 44.1581, -114.1120 | 44.1660, -114.1722 |  |
| 221 | S2B_44RNV_20241002_0_L2A | 80 m | 31.3424, 81.0129 | 31.3178, 81.0232 |  |
| 222 | S2B_26WNE_20240707_0_L2A | 80 m | 71.7255, -24.5766 | 71.7149, -24.5783 |  |
| 223 | S2A_18LZH_20240528_0_L2A | 80 m | -15.9077, -72.0080 | -15.4681, -71.4068 | Colca Canyon, Arequipa Peru (23 km) |
| 224 | S2B_18LUK_20241225_0_L2A | 20 m window | -13.9065, -76.2998 | -13.9285, -76.3739 | Paracas Peninsula (Peru) (8 km) |
| 225 | S2B_11SNA_20241009_0_L2A | 80 m | 37.0159, -116.7695 | 36.8640, -116.8845 |  |
| 226 | S2B_38LPH_20240919_0_L2A | 80 m | -15.6893, 46.7689 | -15.7259, 46.8617 |  |
| 227 | S2A_32TQS_20230910_1_L2A | 80 m | 46.5925, 12.9615 | 46.5238, 12.9498 |  |
| 228 | S2B_54HXF_20241218_0_L2A | 80 m | -35.5988, 142.1939 | -35.5574, 142.2455 |  |
| 229 | S2A_11TMN_20241010_0_L2A | 20 m window | 47.0230, -117.2659 | 46.9921, -117.3021 | Palouse wheat hills, Washington, USA (13 km) |
| 230 | S2B_40SBD_20240518_0_L2A | 80 m | 34.5770, 53.9013 | 34.3458, 54.1131 |  |
| 231 | S2A_44SKA_20240930_0_L2A | 20 m window | 32.1882, 78.0737 | 32.1829, 78.0924 | Spiti Valley, India (8 km) |
| 232 | S2A_11SQA_20240703_0_L2A | 80 m | 36.0563, -114.2764 | 36.1176, -114.3356 |  |
| 233 | S2A_29VPD_20230420_0_L2A | 80 m | 57.3387, -5.6931 | 56.7795, -6.4735 | Cuillin Hills, Isle of Skye, Scotland UK (20 km) |
| 234 | S2B_36KYB_20231106_0_L2A | 80 m | -21.6979, 35.7411 | -20.9768, 35.9544 |  |
| 235 | S2A_49MGM_20230624_0_L2A | 20 m window | -7.9559, 112.9595 | -7.9325, 112.9489 | Bromo-Tengger caldera, Java, Indonesia (1 km) |
| 236 | S2A_58GGR_20241216_0_L2A | 80 m | -44.7910, 168.2712 | -44.7863, 168.3074 |  |
| 237 | S2A_59GMM_20230605_0_L2A | 20 m window | -43.9526, 170.3088 | -43.9562, 170.3170 | Lakes Pukaki and Tekapo, New Zealand (5 km) |
| 238 | S2A_46WEB_20240913_0_L2A | 80 m | 69.1155, 93.4887 | 69.0921, 93.4766 | Putorana Plateau, Siberia Russia (24 km) |
| 239 | S2B_36QVF_20240718_0_L2A | 20 m window | 18.3446, 32.7628 | 18.3576, 32.7599 | Bayuda volcanic field, Sudan (6 km) |
| 240 | S2B_26WPE_20240717_0_L2A | 20 m window | 71.2013, -23.6019 | 71.2013, -23.6160 | Jameson Land, East Greenland (12 km) |
| 241 | S2A_28WDT_20230531_0_L2A | 20 m window | 65.4856, -16.9745 | 65.6039, -16.6169 | Myvatn lava and lake, Iceland (11 km) |
| 242 | S2B_28RFT_20240715_0_L2A | 20 m window | 29.0620, -13.7044 | 29.0756, -13.7365 | Timanfaya, Lanzarote Spain (8 km) |
| 243 | S2A_12SVH_20240620_0_L2A | 80 m | 37.9205, -111.9071 | 37.8595, -112.0246 |  |
| 244 | S2B_36MZB_20240914_0_L2A | 20 m window | -2.7299, 36.0010 | -2.7661, 35.9790 | Ol Doinyo Lengai, Tanzania (9 km) |
| 245 | S2A_37REJ_20240915_0_L2A | 80 m | 25.2479, 39.7926 | 25.3739, 39.7120 |  |
| 246 | S2B_32TMS_20241231_0_L2A | 80 m | 46.0517, 8.5020 | 46.1201, 8.5420 |  |
| 247 | S2B_26WPE_20240717_0_L2A | 20 m window | 71.2685, -23.4797 | 71.2667, -23.4674 | Jameson Land, East Greenland (4 km) |
| 248 | S2A_27WWM_20240707_0_L2A | 80 m | 64.2412, -19.1854 | 64.0964, -18.8131 | Landmannalaugar rhyolite, Iceland (20 km) |
| 249 | S2A_16XDP_20240706_1_L2A | 20 m window | 79.5067, -90.2469 | 79.5032, -90.3394 | Axel Heiberg Island, Nunavut Canada (4 km) |
| 250 | S2A_30SWG_20240621_0_L2A | 20 m window | 37.1974, -2.3693 | 37.1841, -2.3914 | Tabernas Desert, Almeria Spain (16 km) |
| 251 | S2A_34HFK_20240302_0_L2A | 80 m | -32.1731, 23.0396 | -32.1345, 22.5680 |  |
| 252 | S2A_32VMP_20240522_0_L2A | 80 m | 62.1796, 9.0237 | 62.1232, 9.0814 |  |
| 253 | S2B_44RNV_20241002_0_L2A | 20 m window | 30.8244, 81.5485 | 30.8366, 81.5286 | Mount Kailash / Lake Manasarovar, Tibet (9 km) |
| 254 | S2B_33XWH_20230705_1_L2A | 80 m | 78.9983, 19.9370 | 79.0226, 19.8033 |  |
| 255 | S2A_46WEB_20240913_0_L2A | 80 m | 69.3892, 94.5442 | 69.3739, 94.5314 |  |
| 256 | S2B_45VXJ_20240129_0_L2A | 20 m window | 61.4839, 89.9048 | 61.4860, 89.9166 | Yenisei taiga, Russia (5 km) |
| 257 | S2B_19KFP_20240407_0_L2A | 80 m | -23.9605, -67.6538 | -23.9800, -67.6501 | Lagunas Miscanti and Minique, Chile (28 km) |
| 258 | S2B_33XWH_20230705_1_L2A | 80 m | 79.0001, 15.4548 | 79.0031, 15.6437 |  |
| 259 | S2B_21KVA_20240922_0_L2A | 80 m | -17.9514, -57.9190 | -17.6587, -57.6666 |  |
| 260 | S2A_54JTL_20240722_0_L2A | 20 m window | -31.5327, 138.7243 | -31.5287, 138.7281 | Flinders Ranges (Wilpena Pound), Australia (12 km) |
| 261 | S2B_57VXC_20241015_0_L2A | 80 m | 56.0035, 160.9430 | 56.0240, 160.9149 | Klyuchevskaya volcano group, Kamchatka (18 km) |
| 262 | S2B_18GXP_20230619_0_L2A | 20 m window | -46.4920, -72.8427 | -46.5084, -72.8459 | Lago General Carrera, Chile (16 km) |
| 263 | S2A_46WEB_20240913_0_L2A | 80 m | 69.1366, 94.2574 | 69.1542, 94.2164 | Putorana Plateau, Siberia Russia (19 km) |
| 264 | S2A_29VPD_20230420_0_L2A | 20 m window | 57.2994, -6.1313 | 57.2928, -6.1245 | Cuillin Hills, Isle of Skye, Scotland UK (10 km) |
| 265 | S2A_30TXM_20240123_0_L2A | 20 m window | 42.1346, -1.5420 | 42.1387, -1.5452 | Bardenas Reales, Navarre Spain (8 km) |
| 266 | S2B_20TMS_20241019_0_L2A | 20 m window | 46.3987, -63.4836 | 46.4092, -63.5021 | Prince Edward Island (7 km) |
| 267 | S2A_27WWM_20240707_0_L2A | 80 m | 64.5838, -18.7845 | 64.5490, -19.0668 |  |
| 268 | S2A_37PFR_20240909_0_L2A | 20 m window | 13.6134, 40.6665 | 13.6585, 40.6226 | Erta Ale volcano, Ethiopia (5 km) |
| 269 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.9044, 17.7317 | 24.9168, 17.7457 | Waw an Namus (Libya) (2 km) |
| 270 | S2A_33XWG_20240718_2_L2A | 80 m | 78.0482, 17.9263 | 78.0145, 17.7578 |  |
| 271 | S2A_44SKA_20240930_0_L2A | 80 m | 32.3236, 78.5336 | 32.3312, 78.5589 |  |
| 272 | S2B_45VXJ_20240129_0_L2A | 80 m | 61.4542, 89.4238 | 61.5310, 88.9004 |  |
| 273 | S2B_45VXJ_20240129_0_L2A | 20 m window | 61.5561, 89.9484 | 61.5096, 89.9521 | Yenisei taiga, Russia (5 km) |
| 274 | S2B_50RMT_20241106_0_L2A | 80 m | 29.4961, 116.2214 | 29.6201, 116.5329 |  |
| 275 | S2B_37PHN_20241130_0_L2A | 80 m | 10.8701, 41.7536 | 10.7927, 41.7916 |  |
| 276 | S2B_52JFT_20240917_0_L2A | 80 m | -24.6864, 130.1188 | -24.5557, 130.0694 |  |
| 277 | S2B_26WNE_20240707_0_L2A | 80 m | 71.9245, -24.3967 | 71.8997, -24.4035 |  |
| 278 | S2B_26WPE_20240717_0_L2A | 80 m | 71.1584, -23.5837 | 71.1369, -23.5535 | Jameson Land, East Greenland (17 km) |
| 279 | S2A_32VMP_20240919_0_L2A | 80 m | 61.4495, 8.8938 | 61.4449, 8.9397 |  |
| 280 | S2A_33KVQ_20241010_0_L2A | 20 m window | -23.0727, 14.4249 | -23.0864, 14.4215 | Walvis Bay salt works and Sandwich Harbour (9 km) |
| 281 | S2A_28WDS_20240614_0_L2A | 20 m window | 64.0091, -16.3441 | 63.9880, -16.3411 | Jokulsarlon and Breidamerkurjokull, Iceland (7 km) |
| 282 | S2A_43SFV_20240317_0_L2A | 80 m | 35.7212, 77.0216 | 35.7241, 76.9797 |  |
| 283 | S2B_16QDE_20240410_0_L2A | 20 m window | 17.2531, -87.8638 | 17.2411, -87.8570 | Turneffe Atoll, Belize (11 km) |
| 284 | S2B_38PKT_20240603_0_L2A | 80 m | 11.5037, 42.5439 | 11.4821, 42.5651 | Lake Assal, Djibouti (23 km) |
| 285 | S2A_35KLT_20240730_0_L2A | 80 m | -20.1460, 25.4998 | -20.4243, 25.2685 |  |
| 286 | S2A_33RYH_20240611_0_L2A | 20 m window | 24.9508, 17.7646 | 24.9588, 17.7878 | Waw an Namus, Libya (4 km) |
| 287 | S2A_50SPG_20240425_0_L2A | 20 m window | 37.7310, 119.1347 | 37.7354, 119.1264 | Yellow River Delta, China (3 km) |
| 288 | S2A_43SFV_20240923_0_L2A | 80 m | 35.7036, 76.9483 | 35.6739, 76.9552 |  |
| 289 | S2B_10TES_20241107_0_L2A | 80 m | 46.3359, -121.9704 | 46.3522, -121.8732 | Mount St Helens, USA (26 km) |
| 290 | S2B_38SNG_20240706_0_L2A | 20 m window | 37.3055, 45.5576 | 37.3000, 45.5350 | Lake Urmia, Iran (12 km) |
| 291 | S2B_13TGJ_20241004_0_L2A | 80 m | 43.3282, -102.1238 | 43.2989, -102.0977 |  |
| 292 | S2A_18FXK_20240516_0_L2A | 80 m | -50.0642, -72.3805 | -50.0869, -72.3628 |  |
| 293 | S2B_38PKT_20240603_0_L2A | 80 m | 11.6587, 42.7945 | 11.4743, 42.5072 | Lake Assal, Djibouti (27 km) |
| 294 | S2A_57VXC_20240423_0_L2A | 20 m window | 56.0609, 160.6706 | 56.0476, 160.6221 | Klyuchevskaya volcano group, Kamchatka (1 km) |
| 295 | S2B_39TUL_20241123_0_L2A | 20 m window | 45.8436, 48.8676 | 45.8215, 48.8149 | Volga Delta, Russia (11 km) |
| 296 | S2B_12VVJ_20230829_0_L2A | 20 m window | 57.0215, -111.4810 | 57.0087, -111.4769 | Athabasca oil sands mines, Alberta Canada (5 km) |
| 297 | S2B_52KDA_20240314_0_L2A | 20 m window | -22.5522, 128.7364 | -22.5514, 128.6991 | Lake Mackay, Australia (6 km) |
| 298 | S2B_5VLE_20240615_0_L2A | 20 m window | 58.4333, -155.1294 | 58.4246, -155.1208 | Valley of Ten Thousand Smokes, Alaska USA (13 km) |
| 299 | S2A_16QDF_20240205_0_L2A | 20 m window | 18.5963, -87.2902 | 18.6005, -87.2838 | Banco Chinchorro, Mexico (7 km) |
| 300 | S2B_45VXJ_20240129_0_L2A | 80 m | 61.4506, 90.6272 | 61.4834, 90.7682 |  |
| 301 | S2B_45VXJ_20240129_0_L2A | 80 m | 61.9503, 89.7997 | 61.9733, 89.7883 |  |
| 302 | S2A_40SBD_20240910_0_L2A | 20 m window | 34.5082, 54.2165 | 34.5018, 54.2567 | Dasht-e Kavir (Iran) (12 km) |
| 303 | S2A_20KKU_20241115_0_L2A | 80 m | -24.3322, -65.8697 | -24.4692, -65.9030 |  |
| 304 | S2A_43TDM_20240621_0_L2A | 20 m window | 46.3976, 74.8979 | 46.3946, 74.8892 | Lake Balkhash, Kazakhstan (14 km) |
| 305 | S2A_37REJ_20241005_0_L2A | 20 m window | 25.7475, 39.8840 | 25.6894, 39.8401 | Harrat Khaybar lava field (Saudi Arabia) (4 km) |
| 306 | S2A_46RBT_20240309_0_L2A | 80 m | 28.9824, 90.5133 | 29.0014, 90.4899 | Yamdrok Tso, Tibet (20 km) |
| 307 | S2B_12SUG_20241003_0_L2A | 20 m window | 37.2798, -113.0423 | 37.2775, -113.0242 | Zion Canyon, Utah USA (4 km) |
| 308 | S2A_32VMP_20240919_0_L2A | 20 m window | 61.6514, 8.2290 | 61.6355, 8.2189 | Jotunheimen, Norway (10 km) |
| 309 | S2A_18FXK_20240516_0_L2A | 20 m window | -50.5121, -72.9426 | -50.5075, -72.9409 | Perito Moreno and Lago Argentino, Patagonia (8 km) |
| 310 | S2C_19JFG_20241115_1_L2A | 80 m | -30.4175, -66.8282 | -30.4631, -66.8865 |  |
| 311 | S2B_33PVQ_20241113_0_L2A | 80 m | 12.7147, 14.6083 | 12.7197, 14.6231 |  |
| 312 | S2B_11TQJ_20240830_0_L2A | 20 m window | 43.3340, -113.5221 | 43.3261, -113.5449 | Craters of the Moon lava field, Idaho USA (3 km) |
| 313 | S2A_5QKC_20241003_0_L2A | 80 m | 20.6574, -154.9121 | 20.6312, -154.9027 |  |
| 314 | S2A_27WWM_20240707_0_L2A | 20 m window | 63.9888, -19.0053 | 63.9912, -19.0136 | Landmannalaugar rhyolite, Iceland (2 km) |
| 315 | S2B_33XWH_20230705_1_L2A | 20 m window | 78.5681, 16.2275 | 78.5536, 16.2950 | Billefjorden, Svalbard (7 km) |
| 316 | S2B_38UPU_20240520_0_L2A | 80 m | 48.4358, 47.2284 | 48.3678, 47.0881 |  |
| 317 | S2A_32VLP_20230421_0_L2A | 80 m | 62.0593, 6.0537 | 62.0748, 6.0916 |  |
| 318 | S2A_38LPQ_20240530_0_L2A | 20 m window | -9.3471, 46.4681 | -9.3500, 46.4713 | Aldabra Atoll, Seychelles (15 km) |
| 319 | S2A_13RBL_20240929_0_L2A | 20 m window | 27.5700, -107.9372 | 27.5699, -107.9321 | Copper Canyon, Chihuahua Mexico (15 km) |
| 320 | S2A_46RBT_20240309_0_L2A | 80 m | 28.8795, 90.6436 | 28.8284, 90.7085 | Yamdrok Tso, Tibet (11 km) |
| 321 | S2B_33XWH_20230705_1_L2A | 80 m | 78.9934, 16.7147 | 79.0306, 16.5927 |  |
| 322 | S2B_11TQJ_20240402_0_L2A | 20 m window | 43.3412, -113.3953 | 43.3409, -113.3883 | Craters of the Moon lava field, Idaho USA (13 km) |
| 323 | S2A_18LZH_20240528_0_L2A | 20 m window | -15.5072, -72.0124 | -15.5020, -72.0191 | Colca Canyon, Arequipa Peru (16 km) |
| 324 | S2A_31TGJ_20241229_0_L2A | 20 m window | 43.7722, 5.8722 | 43.7757, 5.8769 | Valensole Plateau, Provence (13 km) |
| 325 | S2A_30TXM_20240123_0_L2A | 80 m | 42.2498, -0.7848 | 42.2311, -0.8192 |  |
| 326 | S2B_44TLP_20241008_0_L2A | 80 m | 43.4658, 78.6005 | 43.4377, 78.7123 |  |
| 327 | S2B_23LMG_20240920_0_L2A | 80 m | -12.5054, -45.3157 | -12.4719, -45.3460 |  |
| 328 | S2B_19KEQ_20240914_0_L2A | 20 m window | -22.8231, -68.3594 | -22.8233, -68.3715 | Valle de la Luna / Cordillera de la Sal, Chile (14 km) |
| 329 | S2A_28WDT_20230531_0_L2A | 80 m | 65.1342, -16.3097 | 65.1023, -16.2173 | Askja and Holuhraun lava field, Iceland (26 km) |
| 330 | S2A_33SWB_20241015_0_L2A | 20 m window | 37.8212, 15.2190 | 37.8253, 15.2247 | Mount Etna, Sicily Italy (21 km) |
| 331 | S2B_12VVJ_20231217_0_L2A | 80 m | 56.7937, -111.5722 | 56.8070, -111.5867 | Athabasca oil sands (Canada) (22 km) |
| 332 | S2A_30VVJ_20230226_0_L2A | 80 m | 56.9778, -3.6284 | 56.9698, -3.5853 | Cairngorms heather moorland, Scotland (9 km) |
| 333 | S2B_28WDT_20231030_0_L2A | 80 m | 65.3221, -14.9437 | 65.3070, -15.0345 |  |
| 334 | S2B_50KMC_20240208_0_L2A | 80 m | -20.7789, 116.6366 | -20.7464, 116.6292 | Dampier salt ponds, Pilbara (23 km) |
| 335 | S2B_54HXF_20240422_0_L2A | 80 m | -35.3152, 142.8349 | -35.3379, 142.9395 | Lake Tyrrell, Victoria (5 km) |
| 336 | S2A_20KKU_20240718_0_L2A | 80 m | -23.6677, -65.2421 | -23.6817, -65.2579 |  |
| 337 | S2B_26WNE_20240707_0_L2A | 20 m window | 71.5975, -25.7999 | 71.5975, -25.8166 | Scoresby Sund inner fjords, Greenland (15 km) |
| 338 | S2B_11SQR_20241003_0_L2A | 20 m window | 31.7812, -114.7033 | 31.7851, -114.7267 | Colorado River delta, Mexico (5 km) |
| 339 | S2B_13RGK_20241008_0_L2A | 20 m window | 26.9715, -102.1960 | 26.9531, -102.2053 | Cuatro Cienegas gypsum dunes (Mexico) (16 km) |
| 340 | S2B_48PXR_20230313_0_L2A | 20 m window | 9.7149, 106.5521 | 9.7202, 106.5498 | Mekong River mouths, Vietnam (14 km) |
| 341 | S2B_44TLP_20240809_0_L2A | 80 m | 43.2641, 78.8996 | 43.2834, 78.7484 | Charyn Canyon, Kazakhstan (22 km) |
| 342 | S2A_44SKA_20240930_0_L2A | 80 m | 32.0569, 78.1990 | 32.0895, 78.2047 | Spiti Valley, India (24 km) |
| 343 | S2A_34HFK_20240302_0_L2A | 80 m | -31.8917, 23.0053 | -31.7685, 22.6934 |  |
| 344 | S2B_36MZB_20240914_0_L2A | 80 m | -2.9795, 36.2049 | -2.9779, 36.0311 |  |
| 345 | S2A_37REJ_20241005_0_L2A | 20 m window | 25.8485, 39.9188 | 25.8273, 39.9176 | Harrat Khaybar, Saudi Arabia (5 km) |
| 346 | S2C_53KKP_20241108_1_L2A | 80 m | -24.2079, 132.0780 | -24.1772, 132.1278 |  |
| 347 | S2A_12SWG_20240617_0_L2A | 80 m | 37.5612, -109.9483 | 37.5528, -109.8826 |  |
| 348 | S2B_12SUE_20240615_0_L2A | 80 m | 35.6765, -112.0773 | 35.6516, -112.1019 |  |
| 349 | S2A_13RGK_20240327_0_L2A | 80 m | 26.6921, -101.8925 | 26.6865, -102.1009 | Cuatro Cienegas gypsum dunes (Mexico) (21 km) |
| 350 | S2B_34KFD_20241026_0_L2A | 80 m | -19.3034, 22.5867 | -19.2192, 22.4156 |  |
| 351 | S2B_38UPU_20240907_0_L2A | 80 m | 48.4196, 46.7033 | 48.3865, 46.8240 | Lake Baskunchak, Russia (24 km) |
| 352 | S2A_38UPV_20240925_0_L2A | 20 m window | 49.2197, 46.7547 | 49.1807, 46.7972 | Lake Elton, Russia (11 km) |
| 353 | S2A_30SWG_20240202_0_L2A | 80 m | 37.1285, -2.2792 | 37.1399, -2.2435 | Tabernas Desert, Almeria Spain (15 km) |
| 354 | S2B_10TEN_20240610_0_L2A | 20 m window | 43.0130, -122.0908 | 43.0289, -122.1092 | Crater Lake, Oregon, USA (9 km) |
| 355 | S2A_52KDF_20240408_0_L2A | 80 m | -17.9166, 128.4538 | -17.9193, 128.3945 |  |
| 356 | S2A_36NZJ_20240126_0_L2A | 80 m | 3.5219, 35.7472 | 3.4638, 35.7826 |  |
| 357 | S2A_30SWG_20240202_0_L2A | 80 m | 37.3525, -2.7073 | 37.3101, -2.6292 |  |
| 358 | S2B_54HXF_20241218_0_L2A | 80 m | -35.5583, 142.2828 | -35.4514, 142.3251 |  |
| 359 | S2B_18LZH_20241020_0_L2A | 80 m | -15.5551, -72.0288 | -15.7760, -71.8360 | Colca Canyon, Arequipa Peru (8 km) |
| 360 | S2A_47SND_20240926_0_L2A | 80 m | 38.8612, 99.8377 | 38.8523, 99.8703 | Zhangye Danxia, Gansu (21 km) |
| 361 | S2A_38SLH_20240428_0_L2A | 80 m | 38.5197, 43.6082 | 38.3895, 43.5658 |  |
| 362 | S2A_19JFM_20240519_0_L2A | 80 m | -26.0077, -67.8831 | -25.8996, -67.8323 |  |
| 363 | S2A_50SPG_20240425_0_L2A | 20 m window | 37.7696, 119.1557 | 37.8119, 119.2729 | Yellow River Delta, China (7 km) |
| 364 | S2B_36MZB_20240914_0_L2A | 20 m window | -2.7158, 35.8221 | -2.8730, 35.8599 | Ol Doinyo Lengai, Tanzania (9 km) |
| 365 | S2A_45TYH_20240623_0_L2A | 20 m window | 42.7786, 89.5990 | 42.7948, 89.6635 | Flaming Mountains, Turpan (13 km) |
| 366 | S2A_13RBL_20240522_0_L2A | 20 m window | 27.4508, -107.8920 | 27.4778, -107.9127 | Copper Canyon, Chihuahua Mexico (11 km) |
| 367 | S2A_37TGJ_20240401_0_L2A | 80 m | 43.6027, 42.0050 | 43.5289, 42.1902 |  |
| 368 | S2A_47UPS_20240402_0_L2A | 80 m | 51.1440, 101.3960 | 50.9876, 101.1721 |  |
| 369 | S2B_18LZH_20241020_0_L2A | 20 m window | -15.5479, -71.9849 | -15.5390, -71.9908 | Colca Canyon, Arequipa Peru (11 km) |
| 370 | S2A_30VUH_20230420_0_L2A | 20 m window | 56.5810, -4.8826 | 56.5842, -4.8956 | Rannoch Moor, Scotland, UK (11 km) |
| 371 | S2B_37PDQ_20241116_0_L2A | 80 m | 12.9129, 38.6340 | 12.9212, 38.7762 |  |
| 372 | S2A_36MZB_20230128_0_L2A | 20 m window | -2.8883, 35.9915 | -2.8728, 35.9542 | Ol Doinyo Lengai, Tanzania (15 km) |
| 373 | S2A_34KFD_20240214_0_L2A | 20 m window | -19.2398, 22.8745 | -19.2499, 22.8637 | Okavango Delta, Botswana (7 km) |
| 374 | S2B_36SXH_20240621_0_L2A | 80 m | 38.2360, 34.4779 | 38.3701, 34.9617 |  |
| 375 | S2A_13RGK_20240327_0_L2A | 20 m window | 26.7458, -102.0463 | 26.7975, -102.1421 | Cuatro Cienegas gypsum dunes (Mexico) (9 km) |
| 376 | S2B_40TEQ_20240418_0_L2A | 20 m window | 44.8835, 58.0514 | 44.8903, 58.0461 | Aral Sea west basin, Aralkum (27 km) |
| 377 | S2A_20KKU_20240718_0_L2A | 80 m | -23.6677, -65.2421 | -23.6817, -65.2579 |  |
| 378 | S2B_26WNE_20240707_0_L2A | 20 m window | 71.5975, -25.7999 | 71.5975, -25.8166 | Scoresby Sund inner fjords, Greenland (15 km) |
| 379 | S2B_11SQR_20241003_0_L2A | 20 m window | 31.7812, -114.7033 | 31.7851, -114.7267 | Colorado River delta, Mexico (5 km) |
| 380 | S2A_11SMA_20240616_0_L2A | 80 m | 36.4785, -117.4285 | 36.9035, -117.5331 |  |
| 381 | S2B_13TGG_20240606_0_L2A | 80 m | 42.2156, -101.6893 | 42.1863, -101.7162 | Nebraska Sand Hills, USA (27 km) |
| 382 | S2A_33SWB_20241015_0_L2A | 80 m | 37.1612, 15.0798 | 37.1109, 15.1245 |  |
| 383 | S2B_52KDF_20241030_0_L2A | 80 m | -17.9295, 128.7970 | -17.7389, 128.7727 |  |
| 384 | S2B_13RGK_20241008_0_L2A | 20 m window | 26.9715, -102.1960 | 26.9531, -102.2053 | Cuatro Cienegas gypsum dunes (Mexico) (16 km) |
| 385 | S2A_50KNV_20240321_0_L2A | 20 m window | -22.7846, 117.8609 | -22.6813, 117.9076 | Hamersley Range (Australia) (9 km) |
| 386 | S2A_34HFK_20240302_0_L2A | 80 m | -31.8917, 23.0053 | -31.7685, 22.6934 |  |
| 387 | S2B_36MZB_20240914_0_L2A | 80 m | -2.9795, 36.2049 | -2.9779, 36.0311 |  |
| 388 | S2A_37REJ_20241005_0_L2A | 20 m window | 25.8485, 39.9188 | 25.8273, 39.9176 | Harrat Khaybar, Saudi Arabia (5 km) |
| 389 | S2C_53KKP_20241108_1_L2A | 80 m | -24.2079, 132.0780 | -24.1772, 132.1278 |  |
| 390 | S2A_12SWG_20240617_0_L2A | 80 m | 37.5612, -109.9483 | 37.5528, -109.8826 |  |
| 391 | S2A_13RGK_20240327_0_L2A | 80 m | 26.6921, -101.8925 | 26.6865, -102.1009 | Cuatro Cienegas gypsum dunes (Mexico) (21 km) |
| 392 | S2B_34KFD_20241026_0_L2A | 80 m | -19.3034, 22.5867 | -19.2192, 22.4156 |  |
| 393 | S2B_38UPU_20240907_0_L2A | 80 m | 48.4196, 46.7033 | 48.3865, 46.8240 | Lake Baskunchak, Russia (24 km) |
| 394 | S2A_38UPV_20240925_0_L2A | 20 m window | 49.2197, 46.7547 | 49.1807, 46.7972 | Lake Elton, Russia (11 km) |
| 395 | S2A_36NZJ_20240704_0_L2A | 80 m | 3.2464, 36.3925 | 3.2442, 36.3634 |  |
| 396 | S2A_30SWG_20240202_0_L2A | 80 m | 37.1285, -2.2792 | 37.1399, -2.2435 | Tabernas Desert, Almeria Spain (15 km) |
| 397 | S2B_10TEN_20240610_0_L2A | 20 m window | 43.0130, -122.0908 | 43.0289, -122.1092 | Crater Lake, Oregon, USA (9 km) |
| 398 | S2A_52KDF_20240408_0_L2A | 80 m | -17.9166, 128.4538 | -17.9193, 128.3945 |  |
| 399 | S2A_37REJ_20241005_0_L2A | 20 m window | 25.8161, 39.7985 | 25.8151, 39.8038 | Harrat Khaybar, Saudi Arabia (10 km) |
| 400 | S2A_53KKP_20240412_0_L2A | 80 m | -24.1441, 132.4829 | -24.0794, 132.6030 |  |
| 401 | S2B_48TUP_20240521_0_L2A | 80 m | 43.5352, 103.5011 | 43.7092, 103.2683 |  |
| 402 | S2A_30SWG_20240202_0_L2A | 80 m | 37.3525, -2.7073 | 37.3101, -2.6292 |  |
| 403 | S2B_54HXF_20241218_0_L2A | 80 m | -35.5583, 142.2828 | -35.4514, 142.3251 |  |
| 404 | S2A_47SND_20240926_0_L2A | 80 m | 38.8612, 99.8377 | 38.8523, 99.8703 | Zhangye Danxia, Gansu (21 km) |
| 405 | S2A_38SLH_20240428_0_L2A | 80 m | 38.5197, 43.6082 | 38.3895, 43.5658 |  |
| 406 | S2A_19JFM_20240519_0_L2A | 80 m | -26.0077, -67.8831 | -25.8996, -67.8323 |  |
| 407 | S2B_36MZB_20240914_0_L2A | 20 m window | -2.7158, 35.8221 | -2.8730, 35.8599 | Ol Doinyo Lengai, Tanzania (9 km) |
| 408 | S2A_45TYH_20240623_0_L2A | 20 m window | 42.7786, 89.5990 | 42.7948, 89.6635 | Flaming Mountains, Turpan (13 km) |
| 409 | S2A_13RBL_20240522_0_L2A | 20 m window | 27.4508, -107.8920 | 27.4778, -107.9127 | Copper Canyon, Chihuahua Mexico (11 km) |
| 410 | S2A_37TGJ_20240401_0_L2A | 80 m | 43.6027, 42.0050 | 43.5289, 42.1902 |  |
| 411 | S2A_47UPS_20240402_0_L2A | 80 m | 51.1440, 101.3960 | 50.9876, 101.1721 |  |
| 412 | S2B_18LZH_20241020_0_L2A | 20 m window | -15.5479, -71.9849 | -15.5390, -71.9908 | Colca Canyon, Arequipa Peru (11 km) |
| 413 | S2A_30VUH_20230420_0_L2A | 20 m window | 56.5810, -4.8826 | 56.5842, -4.8956 | Rannoch Moor, Scotland, UK (11 km) |
| 414 | S2B_37PDQ_20241116_0_L2A | 80 m | 12.9129, 38.6340 | 12.9212, 38.7762 |  |
| 415 | S2A_36MZB_20230128_0_L2A | 20 m window | -2.8883, 35.9915 | -2.8728, 35.9542 | Ol Doinyo Lengai, Tanzania (15 km) |
| 416 | S2A_34KFD_20240214_0_L2A | 20 m window | -19.2398, 22.8745 | -19.2499, 22.8637 | Okavango Delta, Botswana (7 km) |
| 417 | S2A_29VPD_20230420_0_L2A | 80 m | 56.8460, -6.6712 | 56.9492, -6.4350 |  |
| 418 | S2A_37QBD_20240617_0_L2A | 20 m window | 21.0763, 37.1452 | 21.0683, 37.1543 | Dungonab Bay, Sudan (3 km) |
| 419 | S2B_18FXJ_20240819_0_L2A | 20 m window | -51.1193, -72.8046 | -51.1105, -72.8160 | Torres del Paine, Chile (15 km) |
| 420 | S2B_36KYB_20231106_0_L2A | 80 m | -21.0402, 35.7719 | -21.0571, 35.9209 |  |
| 421 | S2B_36KYB_20231106_0_L2A | 80 m | -21.6098, 35.8952 | -21.5720, 35.9005 |  |
| 422 | S2B_36MXC_20240828_0_L2A | 20 m window | -2.5482, 34.6614 | -2.5527, 34.6996 | Serengeti plains, Tanzania (14 km) |
| 423 | S2B_48RVN_20230131_0_L2A | 20 m window | 24.9873, 104.1924 | 24.9880, 104.2033 | Luoping rapeseed fields, Yunnan (14 km) |
| 424 | S2A_57VXC_20240423_0_L2A | 20 m window | 56.0387, 160.6623 | 56.0313, 160.6563 | Klyuchevskaya volcano group, Kamchatka (2 km) |
| 425 | S2B_8WMB_20241004_0_L2A | 20 m window | 69.0671, -134.9673 | 69.0570, -134.9424 | Mackenzie River Delta, Canada (7 km) |
| 426 | S2A_53KKP_20240412_0_L2A | 20 m window | -23.7323, 132.7666 | -23.7428, 132.7556 | MacDonnell Ranges, Australia (7 km) |
| 427 | S2B_54HXF_20240422_0_L2A | 20 m window | -35.3931, 142.7283 | -35.3913, 142.6981 | Lake Tyrrell, Victoria (13 km) |
| 428 | S2B_19KFP_20240407_0_L2A | 20 m window | -23.8360, -67.6644 | -23.7875, -67.6793 | Lagunas Miscanti and Minique, Chile (13 km) |
| 429 | S2A_38UPV_20240925_0_L2A | 80 m | 49.4470, 47.3315 | 49.5993, 47.4948 |  |
| 430 | S2A_30VUH_20230420_0_L2A | 80 m | 56.7910, -6.0968 | 56.7309, -5.7824 |  |
| 431 | S2A_37MCS_20230128_0_L2A | 20 m window | -3.1008, 37.4005 | -3.1296, 37.3930 | Kilimanjaro, Tanzania (7 km) |
| 432 | S2B_49KGP_20240809_0_L2A | 80 m | -24.0072, 114.0067 | -24.0003, 113.9818 |  |
| 433 | S2A_51KWT_20240204_0_L2A | 20 m window | -20.4352, 123.9746 | -20.4428, 123.9791 | Great Sandy Desert, Australia (7 km) |
| 434 | S2B_50KQV_20240323_0_L2A | 20 m window | -23.2926, 119.5642 | -23.3030, 119.5641 | Mount Whaleback iron mine, Australia (13 km) |
| 435 | S2A_50KQV_20241123_0_L2A | 80 m | -22.6075, 119.9604 | -22.6325, 119.9427 |  |
| 436 | S2A_10TEN_20241003_0_L2A | 80 m | 43.3413, -121.7109 | 43.3225, -122.0140 |  |
| 437 | S2A_36MYB_20230720_0_L2A | 80 m | -2.8512, 35.1433 | -2.8465, 35.1258 |  |
| 438 | S2B_52KDA_20240314_0_L2A | 80 m | -21.9763, 128.4014 | -21.9610, 128.3962 |  |
| 439 | S2B_11SLC_20240604_0_L2A | 20 m window | 37.9445, -118.8574 | 38.0990, -118.8818 | Mono Lake, California (12 km) |
| 440 | S2A_12SVH_20240620_0_L2A | 80 m | 38.5915, -111.8403 | 38.6898, -111.8917 |  |
| 441 | S2A_28RCS_20240414_0_L2A | 20 m window | 28.2903, -16.7135 | 28.3567, -16.6736 | Teide caldera, Tenerife Spain (8 km) |
| 442 | S2C_19JFG_20241115_1_L2A | 80 m | -30.5085, -66.9364 | -30.5331, -67.2486 |  |
| 443 | S2A_30TXM_20240123_0_L2A | 20 m window | 42.1929, -1.6491 | 42.1893, -1.6536 | Bardenas Reales, Navarre Spain (13 km) |
| 444 | S2B_36SWH_20241009_0_L2A | 80 m | 38.6941, 34.1928 | 38.6937, 34.1650 |  |
| 445 | S2B_52KDA_20240314_0_L2A | 20 m window | -22.6004, 128.6406 | -22.6199, 128.5926 | Lake Mackay (Australia) (12 km) |
| 446 | S2B_45QYE_20240430_0_L2A | 80 m | 22.1924, 89.6190 | 22.1749, 89.5949 |  |
| 447 | S2B_13RGK_20241008_0_L2A | 80 m | 26.1377, -102.5546 | 26.3006, -102.5529 |  |
| 448 | S2A_36SXH_20241004_0_L2A | 80 m | 38.6782, 34.5092 | 38.6249, 34.5492 | Cappadocia, Turkey (28 km) |
| 449 | S2B_11SQA_20240320_0_L2A | 80 m | 36.6192, -114.2138 | 36.5262, -114.2401 | Valley of Fire / Lake Mead, Nevada USA (28 km) |
| 450 | S2B_11TLF_20240316_0_L2A | 80 m | 41.1553, -118.4248 | 41.1362, -118.4565 |  |
| 451 | S2A_12SVH_20240620_0_L2A | 80 m | 38.3784, -111.7566 | 38.3386, -111.8598 |  |
| 452 | S2A_32VMP_20240919_0_L2A | 20 m window | 61.5229, 8.6315 | 61.5284, 8.6367 | Jotunheimen, Norway (15 km) |
| 453 | S2A_12SXD_20240617_0_L2A | 80 m | 34.9548, -109.3247 | 34.7733, -109.3769 |  |
| 454 | S2B_37PHN_20241130_0_L2A | 20 m window | 11.5473, 42.4603 | 11.5319, 42.4573 | Lake Assal, Djibouti (13 km) |
| 455 | S2B_36MXC_20240828_0_L2A | 80 m | -2.6462, 34.6966 | -2.5226, 34.8569 | Serengeti plains, Tanzania (10 km) |
| 456 | S2B_38UPU_20240520_0_L2A | 20 m window | 48.0925, 46.9319 | 48.0851, 46.8770 | Lake Baskunchak, Russia (12 km) |
| 457 | S2A_34HFK_20240302_0_L2A | 80 m | -31.7655, 22.9205 | -31.7798, 22.9667 |  |
| 458 | S2B_28RFT_20240715_0_L2A | 20 m window | 28.9559, -13.6727 | 28.9632, -13.6754 | Timanfaya, Lanzarote Spain (9 km) |
| 459 | S2A_54JTL_20241219_0_L2A | 20 m window | -31.6555, 138.6950 | -31.6235, 138.7160 | Flinders Ranges (Wilpena Pound), Australia (14 km) |
| 460 | S2B_11TQJ_20240830_0_L2A | 80 m | 44.0421, -114.1048 | 43.9996, -114.0523 |  |
| 461 | S2A_38UPV_20240925_0_L2A | 80 m | 49.5247, 47.1794 | 49.3230, 47.3500 |  |
| 462 | S2A_38SNG_20241108_0_L2A | 20 m window | 37.3266, 45.7114 | 37.3592, 45.7217 | Lake Urmia, Iran (12 km) |
| 463 | S2B_36SWH_20241009_0_L2A | 80 m | 38.5912, 34.2071 | 38.5469, 34.2259 |  |
| 464 | S2B_44TLP_20240809_0_L2A | 20 m window | 43.4881, 79.1892 | 43.4810, 79.2007 | Charyn Canyon, Kazakhstan (18 km) |
| 465 | S2B_19KFR_20240308_0_L2A | 20 m window | -22.2240, -67.8090 | -22.2544, -67.8155 | Laguna Colorada, Bolivia (5 km) |
| 466 | S2A_19JDM_20240323_0_L2A | 80 m | -25.9716, -69.4833 | -26.0706, -69.5923 |  |
| 467 | S2A_44SKA_20240930_0_L2A | 80 m | 31.7095, 78.4454 | 31.7181, 78.4209 |  |
| 468 | S2B_12UVB_20240721_0_L2A | 20 m window | 50.8491, -111.5705 | 50.8343, -111.5905 | Dinosaur Provincial Park badlands, Alberta Canada (12 km) |
| 469 | S2B_13TGG_20240606_0_L2A | 80 m | 41.7147, -102.2362 | 41.7268, -102.2609 |  |
| 470 | S2A_38UPV_20240925_0_L2A | 20 m window | 49.2072, 46.7975 | 49.2148, 46.7923 | Lake Elton, Russia (13 km) |
| 471 | S2A_12SXD_20240617_0_L2A | 80 m | 35.0271, -109.4199 | 35.1076, -109.0957 |  |
| 472 | S2B_11SQA_20240320_0_L2A | 80 m | 36.0439, -114.2133 | 36.0343, -114.2369 |  |
| 473 | S2A_29VPE_20230420_0_L2A | 20 m window | 57.6603, -7.2200 | 57.6706, -7.2060 | Sound of Harris, Outer Hebrides (13 km) |
| 474 | S2B_51KVB_20240929_0_L2A | 80 m | -16.4544, 122.9730 | -16.4118, 122.9921 | Cape Leveque, Dampier Peninsula (7 km) |
| 475 | S2B_12STA_20240615_0_L2A | 20 m window | 31.7541, -113.4265 | 31.7511, -113.4204 | Pinacate volcanic field, Sonora Mexico (9 km) |
| 476 | S2B_12TTL_20240320_0_L2A | 80 m | 41.1429, -114.1308 | 41.2129, -114.1102 |  |
| 477 | S2A_52KDF_20240408_0_L2A | 80 m | -17.8004, 128.6207 | -17.7891, 128.6343 |  |
| 478 | S2A_36MZB_20230128_0_L2A | 20 m window | -2.7123, 35.8001 | -2.7863, 35.8210 | Ol Doinyo Lengai, Tanzania (11 km) |
| 479 | S2B_36MZD_20240805_0_L2A | 80 m | -1.2165, 36.1047 | -1.3850, 36.3450 |  |
| 480 | S2A_27VXL_20241012_0_L2A | 20 m window | 63.6739, -18.3626 | 63.7208, -18.2819 | Eldhraun moss-covered lava, Iceland (8 km) |
| 481 | S2A_19KHQ_20240320_0_L2A | 80 m | -23.1622, -65.7125 | -23.0484, -65.8999 |  |
| 482 | S2B_42QWM_20241001_0_L2A | 20 m window | 23.8920, 69.8020 | 23.8966, 69.8015 | Great Rann of Kutch (India) (1 km) |
| 483 | S2B_11TQJ_20240830_0_L2A | 80 m | 43.7273, -113.6180 | 43.7449, -113.7340 |  |
| 484 | S2B_13SDT_20240904_0_L2A | 80 m | 33.5791, -105.7679 | 33.7065, -105.5845 | Carrizozo Malpais lava flow, New Mexico USA (26 km) |
| 485 | S2A_14SKG_20241006_0_L2A | 20 m window | 37.6841, -101.3354 | 37.6900, -101.3426 | Southwest Kansas centre pivots, USA (13 km) |
| 486 | S2B_13TGG_20240606_0_L2A | 80 m | 42.1024, -102.3174 | 42.0765, -102.3328 |  |
| 487 | S2B_11SQA_20240320_0_L2A | 80 m | 36.2454, -114.0386 | 36.2744, -114.1506 |  |
| 488 | S2B_23LLG_20240930_0_L2A | 80 m | -11.8547, -46.4474 | -11.8976, -46.4459 |  |
| 489 | S2A_12SUG_20240620_0_L2A | 80 m | 37.8431, -112.5253 | 37.8921, -112.5042 |  |
| 490 | S2B_5VLE_20240615_0_L2A | 80 m | 57.9497, -155.7017 | 57.9420, -155.6729 |  |
| 491 | S2A_18FXJ_20231208_0_L2A | 80 m | -50.9886, -72.9143 | -50.9403, -72.5992 | Torres del Paine, Chile (17 km) |
| 492 | S2A_28RCS_20240414_0_L2A | 20 m window | 28.2293, -16.5505 | 28.1908, -16.5437 | Teide caldera, Tenerife Spain (11 km) |
| 493 | S2A_36NZK_20241231_0_L2A | 80 m | 4.1784, 36.6531 | 4.2332, 36.3540 |  |
| 494 | S2A_45TYH_20241001_0_L2A | 80 m | 42.8764, 90.1284 | 42.8861, 90.0816 |  |
| 495 | S2B_11SQR_20241003_0_L2A | 20 m window | 31.6983, -114.6088 | 31.6450, -114.8607 | Colorado River Delta, Gulf of California (3 km) |
| 496 | S2A_20GMU_20241226_0_L2A | 20 m window | -42.3900, -63.8899 | -42.3206, -63.8894 | Península Valdés (Argentina) (18 km) |
| 497 | S2A_33SWB_20241015_0_L2A | 20 m window | 37.8533, 15.0745 | 37.8528, 15.0414 | Mount Etna, Sicily Italy (13 km) |
| 498 | S2B_40RFU_20241006_0_L2A | 20 m window | 30.3730, 58.9034 | 30.2936, 58.9891 | Dasht-e Lut kaluts, Iran (5 km) |
| 499 | S2B_46TCK_20241003_0_L2A | 80 m | 40.4782, 91.4399 | 40.4711, 91.4929 |  |
| 500 | S2A_40QEL_20240524_0_L2A | 80 m | 23.4891, 57.5585 | 23.4796, 57.5825 | Hajar Mountains ophiolite, Oman (26 km) |
| 501 | S2B_45TYE_20241023_0_L2A | 20 m window | 40.2811, 90.5064 | 40.2682, 90.5220 | Lop Nur salt basin, China (10 km) |
| 502 | S2B_13SDT_20240527_0_L2A | 80 m | 33.8497, -105.5430 | 33.7750, -105.4204 |  |
| 503 | S2B_44TLP_20240809_0_L2A | 80 m | 43.8695, 79.1560 | 43.8955, 79.1944 |  |
| 504 | S2A_36MZB_20230128_0_L2A | 20 m window | -2.7465, 35.9922 | -2.7537, 35.9847 | Ol Doinyo Lengai, Tanzania (9 km) |
| 505 | S2B_46TCK_20241003_0_L2A | 20 m window | 40.2301, 90.7653 | 40.2485, 90.7494 | Lop Nur, Xinjiang (13 km) |
| 506 | S2A_33RYH_20240611_0_L2A | 20 m window | 24.8913, 17.7046 | 24.9053, 17.6934 | Waw an Namus (Libya) (6 km) |
| 507 | S2B_47TQJ_20240623_0_L2A | 80 m | 43.3661, 101.8327 | 43.2616, 101.7607 |  |
| 508 | S2B_19KFR_20240308_0_L2A | 20 m window | -22.2279, -67.8197 | -22.2207, -67.8124 | Laguna Colorada, Bolivia (5 km) |
| 509 | S2B_33JYJ_20240402_0_L2A | 80 m | -28.7972, 17.4936 | -28.7887, 17.5124 |  |
| 510 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.9744, 17.7888 | 24.9817, 17.7943 | Waw an Namus, Libya (7 km) |
| 511 | S2A_19JFM_20240110_0_L2A | 20 m window | -25.6304, -67.6893 | -25.6614, -67.6586 | Salar de Antofalla, Argentina (7 km) |
| 512 | S2B_11TLF_20240316_0_L2A | 20 m window | 40.7898, -119.0450 | 40.7873, -119.0532 | Black Rock Desert playa (USA) (8 km) |
| 513 | S2A_35KLT_20240122_0_L2A | 20 m window | -20.5160, 25.9708 | -20.5498, 25.9871 | Sua Pan, Makgadikgadi, Botswana (11 km) |
| 514 | S2B_11SQR_20241003_0_L2A | 20 m window | 31.7712, -114.7852 | 31.7250, -114.7834 | Colorado River delta, Mexico (3 km) |
| 515 | S2B_38UPU_20240907_0_L2A | 20 m window | 48.1317, 46.7593 | 48.1229, 46.7671 | Lake Baskunchak, Russia (12 km) |
| 516 | S2B_40RFU_20241006_0_L2A | 20 m window | 30.4385, 59.1020 | 30.4028, 59.1169 | Dasht-e Lut kaluts, Iran (10 km) |
| 517 | S2B_45TYE_20241023_0_L2A | 80 m | 39.8671, 90.0965 | 39.7466, 90.1035 |  |
| 518 | S2B_36MZB_20240914_0_L2A | 20 m window | -2.9528, 35.9958 | -2.9541, 36.0008 | Ol Doinyo Lengai, Tanzania (24 km) |
| 519 | S2A_47TQJ_20241006_1_L2A | 80 m | 43.9747, 101.7913 | 43.9674, 101.8127 |  |
| 520 | S2A_20KKU_20240718_0_L2A | 20 m window | -23.5701, -65.7615 | -23.5356, -65.8030 | Salinas Grandes, Argentina (13 km) |
| 521 | S2A_13RGK_20240327_0_L2A | 80 m | 26.5521, -102.0856 | 26.5195, -102.1340 |  |
| 522 | S2B_36MZB_20240914_0_L2A | 80 m | -2.8378, 35.9766 | -2.9734, 36.1738 | Ol Doinyo Lengai, Tanzania (24 km) |
| 523 | S2A_33RYH_20240611_0_L2A | 20 m window | 24.9578, 17.7282 | 24.9350, 17.7501 | Waw an Namus (Libya) (3 km) |
| 524 | S2A_30RTV_20241009_0_L2A | 20 m window | 31.1250, -5.7291 | 31.1093, -5.7882 | Jbel Saghro, Morocco (6 km) |
| 525 | S2A_20GMU_20240420_0_L2A | 20 m window | -42.4536, -64.1568 | -42.4833, -64.1293 | Península Valdés (Argentina) (12 km) |
| 526 | S2B_52KDF_20241030_0_L2A | 80 m | -17.4468, 128.5825 | -17.4931, 128.3024 | Bungle Bungle (Purnululu), Australia (5 km) |
| 527 | S2B_44TLP_20240809_0_L2A | 80 m | 43.4528, 79.2673 | 43.4022, 79.3201 | Charyn Canyon, Kazakhstan (19 km) |
| 528 | S2B_13RGK_20241008_0_L2A | 80 m | 26.2136, -102.5757 | 26.2999, -102.6036 |  |
| 529 | S2A_20KKU_20241115_0_L2A | 80 m | -23.8249, -65.3453 | -23.7777, -65.4222 |  |
| 530 | S2B_13RGK_20241008_0_L2A | 80 m | 26.4759, -102.7093 | 26.6738, -102.9198 |  |
| 531 | S2A_37REJ_20240915_0_L2A | 80 m | 26.0904, 39.9909 | 26.0541, 39.9970 |  |
| 532 | S2B_11SNA_20241009_0_L2A | 80 m | 36.1470, -116.5088 | 36.1243, -116.4828 | Badwater Basin, Death Valley (27 km) |
| 533 | S2B_44TLP_20241008_0_L2A | 80 m | 43.9826, 79.0553 | 44.0370, 79.0660 | Aktau Mountains, Altyn-Emel Kazakhstan (17 km) |
| 534 | S2A_51KWT_20240204_0_L2A | 20 m window | -20.4991, 123.9487 | -20.4906, 123.9561 | Great Sandy Desert, Australia (5 km) |
| 535 | S2A_49KGP_20240107_0_L2A | 80 m | -23.6990, 113.5933 | -23.6995, 113.6222 |  |
| 536 | S2A_19KFQ_20240919_0_L2A | 80 m | -23.0692, -67.9143 | -23.0858, -68.0120 |  |
| 537 | S2B_29QKD_20241007_0_L2A | 20 m window | 21.2071, -11.5182 | 21.2075, -11.5108 | Richat Structure, Mauritania (15 km) |
| 538 | S2B_19KFR_20241004_0_L2A | 80 m | -21.8263, -67.7873 | -21.8713, -67.7562 |  |
| 539 | S2A_12SXH_20240617_0_L2A | 20 m window | 38.2984, -109.6613 | 38.2924, -109.6235 | Canyonlands, Utah (18 km) |
| 540 | S2B_33JYJ_20240402_0_L2A | 20 m window | -28.0561, 17.2645 | -28.0634, 17.2661 | Richtersveld, South Africa (23 km) |
| 541 | S2A_33KVQ_20241010_0_L2A | 80 m | -23.0780, 14.4554 | -23.1301, 14.4577 | Walvis Bay salt works and Sandwich Harbour (5 km) |
| 542 | S2B_48PXR_20230313_0_L2A | 20 m window | 9.5161, 106.3856 | 9.5028, 106.3854 | Mekong River mouths, Vietnam (16 km) |
| 543 | S2A_38UPV_20240528_0_L2A | 20 m window | 49.1037, 46.8143 | 49.0936, 46.7816 | Lake Elton, Russia (11 km) |
| 544 | S2A_34HFK_20240302_0_L2A | 20 m window | -32.2774, 22.5848 | -32.2703, 22.5930 | Great Karoo, South Africa (9 km) |
| 545 | S2B_37PDQ_20241116_0_L2A | 80 m | 13.2726, 38.3053 | 13.2623, 38.3554 | Simien Mountains, Ethiopia (14 km) |
| 546 | S2B_36MZB_20240914_0_L2A | 80 m | -2.8069, 36.0658 | -2.8896, 35.8924 | Ol Doinyo Lengai, Tanzania (12 km) |
| 547 | S2B_50JKR_20240826_0_L2A | 80 m | -26.4993, 114.0493 | -26.6117, 113.9967 | Hamelin Pool, Shark Bay Australia (25 km) |
| 548 | S2B_36RYT_20240608_0_L2A | 80 m | 28.9640, 35.3824 | 28.9857, 35.2938 |  |
| 549 | S2A_33JYJ_20241203_0_L2A | 80 m | -28.8887, 17.1530 | -28.9094, 17.1347 |  |
| 550 | S2B_13TGJ_20241004_0_L2A | 80 m | 43.4133, -102.2327 | 43.4858, -102.1796 |  |
| 551 | S2B_33JYJ_20240402_0_L2A | 20 m window | -28.2485, 17.2976 | -28.2440, 17.3016 | Richtersveld, South Africa (27 km) |
| 552 | S2B_19KFP_20240407_0_L2A | 20 m window | -23.7099, -67.8015 | -23.6623, -67.7709 | Lagunas Miscanti and Minique, Chile (6 km) |
| 553 | S2B_33JYJ_20240402_0_L2A | 80 m | -28.7910, 17.2148 | -28.7913, 17.2692 |  |
| 554 | S2B_52KDA_20240920_0_L2A | 80 m | -22.5224, 128.1720 | -22.4637, 128.1686 |  |
| 555 | S2A_30PVA_20240416_0_L2A | 20 m window | 14.3286, -3.5575 | 14.3253, -3.5664 | Bandiagara Escarpment, Mali (7 km) |
| 556 | S2A_12SXD_20240617_0_L2A | 20 m window | 35.1257, -109.8203 | 35.1310, -109.8478 | Painted Desert / Petrified Forest, Arizona USA (8 km) |
| 557 | S2B_53JQM_20240312_0_L2A | 80 m | -26.1069, 137.6334 | -25.7316, 137.4915 |  |
| 558 | S2A_48TTK_20241013_0_L2A | 80 m | 39.8834, 102.2262 | 39.9222, 102.2607 | Badain Jaran Desert, China (18 km) |
| 559 | S2A_32SMC_20240207_0_L2A | 20 m window | 33.7016, 8.3592 | 33.6988, 8.3339 | Chott el Djerid (Tunisia) (5 km) |
| 560 | S2B_13SCS_20240527_0_L2A | 80 m | 33.1251, -106.9459 | 33.1796, -106.9568 |  |
| 561 | S2A_19JFG_20240419_0_L2A | 80 m | -29.9617, -67.8675 | -30.0039, -67.8482 | Ischigualasto / Talampaya, Argentina (2 km) |
| 562 | S2B_19KFR_20240308_0_L2A | 80 m | -22.1577, -67.4369 | -22.1368, -67.4107 |  |
| 563 | S2B_42SUD_20240602_0_L2A | 20 m window | 34.7565, 67.0927 | 34.7537, 67.1022 | Band-e Amir lakes, Afghanistan (15 km) |
| 564 | S2B_11TLF_20240922_0_L2A | 80 m | 41.1498, -118.4618 | 41.1393, -118.5282 |  |
| 565 | S2B_13SDT_20240904_0_L2A | 20 m window | 33.7713, -105.8124 | 33.7644, -105.8004 | Carrizozo Malpais lava flow, New Mexico USA (15 km) |
| 566 | S2B_19KFP_20241113_0_L2A | 20 m window | -23.7004, -67.8555 | -23.7046, -67.8631 | Lagunas Miscanti and Minique, Chile (10 km) |
| 567 | S2B_12SVG_20240605_0_L2A | 20 m window | 37.1759, -110.9851 | 37.1698, -110.9845 | Lake Powell, Utah-Arizona (8 km) |
| 568 | S2B_50JKP_20240418_0_L2A | 80 m | -28.6743, 114.6967 | -28.7187, 114.7319 |  |
| 569 | S2B_11SNA_20241009_0_L2A | 80 m | 36.8077, -116.1199 | 36.8349, -116.1047 |  |
| 570 | S2B_35QKE_20240816_0_L2A | 20 m window | 21.9072, 24.9505 | 21.9031, 24.9606 | Jebel Uweinat, Egypt/Libya/Sudan (1 km) |
| 571 | S2B_18LZH_20241020_0_L2A | 80 m | -15.8474, -71.8090 | -16.0248, -71.7320 |  |
| 572 | S2A_19KFQ_20240919_0_L2A | 80 m | -23.1173, -67.4556 | -23.1196, -67.6645 |  |
| 573 | S2A_12SWG_20241005_0_L2A | 80 m | 37.8483, -110.5695 | 37.8557, -110.6355 |  |
| 574 | S2B_48TUP_20240521_0_L2A | 80 m | 43.9055, 103.7920 | 43.5679, 103.7296 |  |
| 575 | S2A_20KKU_20241115_0_L2A | 20 m window | -23.5546, -65.7795 | -23.5580, -65.7455 | Salinas Grandes, Argentina (14 km) |
| 576 | S2A_19KEQ_20240103_0_L2A | 80 m | -23.2499, -68.7232 | -23.4747, -68.6223 |  |
| 577 | S2B_38KNA_20241108_0_L2A | 80 m | -22.5224, 45.4775 | -22.5283, 45.5073 | Isalo Massif, Madagascar (15 km) |
| 578 | S2A_19KER_20240123_0_L2A | 80 m | -22.0527, -68.6162 | -21.7889, -68.4024 |  |
| 579 | S2B_46RBU_20241109_0_L2A | 20 m window | 30.5521, 90.6196 | 30.5473, 90.6239 | Nam Co, Tibet (17 km) |
| 580 | S2A_14SKG_20241006_0_L2A | 80 m | 37.0844, -101.7908 | 37.0419, -101.7909 |  |
| 581 | S2A_36MZB_20230128_0_L2A | 80 m | -2.7811, 36.2361 | -2.9320, 35.8691 | Ol Doinyo Lengai, Tanzania (19 km) |
| 582 | S2B_42QWM_20241001_0_L2A | 20 m window | 23.9992, 69.9996 | 23.9702, 69.9968 | Rann of Kutch, India (9 km) |
| 583 | S2B_12SUE_20240615_0_L2A | 80 m | 35.8944, -112.2064 | 35.7399, -112.2632 |  |
| 584 | S2B_19KFP_20241113_0_L2A | 80 m | -24.2587, -67.7376 | -24.2742, -67.7517 |  |
| 585 | S2A_50JKR_20241219_0_L2A | 80 m | -26.7355, 114.1155 | -26.6724, 114.1126 |  |
| 586 | S2B_34QBG_20241011_0_L2A | 80 m | 19.2969, 18.8615 | 19.2655, 18.8167 |  |
| 587 | S2B_12SVF_20241003_0_L2A | 80 m | 36.4600, -111.9336 | 36.4027, -112.0253 |  |
| 588 | S2A_12SXD_20240617_0_L2A | 80 m | 34.7370, -109.8718 | 34.7322, -109.9042 |  |
| 589 | S2A_19JFM_20240110_0_L2A | 20 m window | -25.6895, -67.5458 | -25.7002, -67.5803 | Salar de Antofalla, Argentina (6 km) |
| 590 | S2B_19KFR_20241004_0_L2A | 80 m | -21.8205, -67.6460 | -21.7086, -67.7237 |  |
| 591 | S2A_37RCK_20241005_0_L2A | 80 m | 26.6935, 37.3255 | 26.6330, 37.3659 |  |
| 592 | S2B_48TUP_20240521_0_L2A | 80 m | 44.1491, 102.8022 | 43.9315, 103.4588 |  |
| 593 | S2A_19JFM_20240519_0_L2A | 20 m window | -25.5966, -67.5866 | -25.6279, -67.6443 | Salar de Antofalla, Argentina (4 km) |

### Sand

| # | Scene | Image | Start | End | Near |
| ---: | --- | --- | --- | --- | --- |
| 0 | S2A_43TDM_20241009_0_L2A | 80 m | 45.9861, 74.7763 | 45.9679, 74.8235 |  |
| 1 | S2B_51KVB_20240929_0_L2A | 80 m | -17.0287, 123.0408 | -17.0279, 122.9980 |  |
| 2 | S2B_38UPU_20240907_0_L2A | 20 m window | 48.2375, 46.8131 | 48.2411, 46.8210 | Lake Baskunchak, Russia (6 km) |
| 3 | S2A_33JWN_20240304_0_L2A | 20 m window | -24.7700, 15.3052 | -24.7843, 15.3034 | Sossusvlei dune sea, Namibia (3 km) |
| 4 | S2B_33JYK_20240412_0_L2A | 20 m window | -27.6453, 17.5336 | -27.6390, 17.5430 | Fish River Canyon, Namibia (8 km) |
| 5 | S2B_40RFU_20241006_0_L2A | 20 m window | 30.4452, 59.0246 | 30.4174, 59.0239 | Dasht-e Lut kaluts, Iran (9 km) |
| 6 | S2B_12SVF_20240605_0_L2A | 80 m | 36.9094, -111.7901 | 36.9256, -111.8122 | Vermilion Cliffs / Paria Plateau, Arizona USA (8 km) |
| 7 | S2B_51KVQ_20240906_0_L2A | 20 m window | -23.4580, 122.8715 | -23.4521, 122.8702 | Lake Disappointment (5 km) |
| 8 | S2A_44RNV_20240417_0_L2A | 20 m window | 30.8614, 81.4692 | 30.8639, 81.4833 | Mount Kailash / Lake Manasarovar, Tibet (3 km) |
| 9 | S2A_11UMT_20240324_0_L2A | 80 m | 52.0669, -118.4337 | 52.0408, -118.4131 |  |
| 10 | S2B_40RFU_20241006_0_L2A | 20 m window | 30.4306, 59.1358 | 30.3926, 59.1052 | Dasht-e Lut kaluts, Iran (10 km) |
| 11 | S2B_40RDR_20240909_0_L2A | 80 m | 27.9203, 56.2583 | 27.9086, 56.2422 |  |
| 12 | S2A_45RVM_20241028_0_L2A | 20 m window | 28.1601, 86.7913 | 28.1652, 86.7970 | Everest massif (Nepal/China) (21 km) |
| 13 | S2A_33RYH_20240611_0_L2A | 20 m window | 24.9537, 17.7246 | 24.9219, 17.7310 | Waw an Namus (Libya) (3 km) |
| 14 | S2B_45TYE_20241023_0_L2A | 80 m | 39.8552, 90.0500 | 39.8809, 90.0700 |  |
| 15 | S2B_46TCK_20241003_0_L2A | 80 m | 40.5551, 91.8299 | 40.4516, 91.5270 |  |
| 16 | S2B_52JFT_20240917_0_L2A | 20 m window | -24.8073, 130.4015 | -24.7904, 130.4104 | Lake Amadeus, Northern Territory (11 km) |
| 17 | S2B_44RNV_20241002_0_L2A | 20 m window | 30.8751, 81.3710 | 30.8576, 81.3627 | Mount Kailash / Lake Manasarovar, Tibet (8 km) |
| 18 | S2B_38UPU_20240907_0_L2A | 20 m window | 48.2599, 46.7249 | 48.2781, 46.7231 | Lake Baskunchak, Russia (14 km) |
| 19 | S2A_44SKA_20240513_0_L2A | 20 m window | 32.3090, 77.9839 | 32.3114, 77.9694 | Spiti Valley, India (10 km) |
| 20 | S2A_30TXM_20240711_0_L2A | 80 m | 41.8579, -1.1625 | 41.8625, -1.1355 |  |
| 21 | S2B_47TQJ_20240623_0_L2A | 80 m | 43.2369, 102.3554 | 43.2405, 102.5802 |  |
| 22 | S2B_19KEQ_20240914_0_L2A | 80 m | -23.4491, -68.5015 | -23.5007, -68.3902 | Salar de Atacama lithium ponds, Chile (13 km) |
| 23 | S2B_46SFF_20240818_0_L2A | 80 m | 36.5322, 95.2365 | 36.5001, 95.2420 |  |
| 24 | S2B_47TQJ_20240623_0_L2A | 80 m | 43.3159, 102.6554 | 43.2286, 102.5553 |  |
| 25 | S2B_36TXR_20240714_0_L2A | 20 m window | 45.6841, 34.5425 | 45.6831, 34.5304 | Sivash lagoons, Crimea (9 km) |
| 26 | S2B_13SDT_20240527_0_L2A | 20 m window | 33.7309, -105.9784 | 33.7317, -105.9715 | Carrizozo Malpais lava flow, New Mexico USA (4 km) |
| 27 | S2A_41RQJ_20240515_0_L2A | 20 m window | 25.4660, 65.5681 | 25.4698, 65.5699 | Hingol / Makran ranges, Pakistan (8 km) |
| 28 | S2A_13RBL_20240522_0_L2A | 80 m | 27.9243, -107.0775 | 27.9071, -107.0614 |  |
| 29 | S2B_44SKC_20240508_0_L2A | 20 m window | 33.6709, 78.7081 | 33.6728, 78.7327 | Pangong Tso, Ladakh (11 km) |
| 30 | S2B_38UPU_20240907_0_L2A | 80 m | 48.1930, 46.8023 | 48.2242, 46.8236 | Lake Baskunchak, Russia (5 km) |
| 31 | S2B_19KFP_20240407_0_L2A | 80 m | -23.7281, -67.2469 | -23.7300, -67.2784 |  |
| 32 | S2B_11TLF_20240316_0_L2A | 80 m | 40.6576, -119.0255 | 40.6790, -118.9848 | Black Rock Desert playa (USA) (22 km) |
| 33 | S2A_47SND_20240330_0_L2A | 80 m | 38.8369, 100.0758 | 38.7871, 100.0632 | Zhangye Danxia, Gansu (12 km) |
| 34 | S2B_12SVF_20241003_0_L2A | 80 m | 36.9263, -111.4106 | 36.9013, -111.3924 | Lake Powell, Padre Bay (20 km) |
| 35 | S2A_44RNV_20240417_0_L2A | 20 m window | 30.8277, 81.5398 | 30.8404, 81.5440 | Mount Kailash / Lake Manasarovar, Tibet (9 km) |
| 36 | S2A_46RBU_20240309_0_L2A | 20 m window | 30.7049, 90.5846 | 30.7082, 90.5885 | Nam Co, Tibet (1 km) |
| 37 | S2A_47TQJ_20241006_1_L2A | 80 m | 44.1529, 101.8757 | 44.1376, 101.8757 |  |
| 38 | S2B_11SLC_20240604_0_L2A | 80 m | 38.0389, -118.5374 | 38.0714, -118.5188 |  |
| 39 | S2A_45RVM_20241028_0_L2A | 80 m | 28.4314, 86.4447 | 28.4460, 86.4263 |  |
| 40 | S2A_13RBL_20240522_0_L2A | 20 m window | 27.4890, -107.8802 | 27.4938, -107.8761 | Copper Canyon, Chihuahua Mexico (8 km) |
| 41 | S2A_44SPJ_20240629_0_L2A | 20 m window | 39.0846, 82.9080 | 39.0900, 82.8913 | Taklamakan Desert, China (13 km) |
| 42 | S2B_48TUP_20240521_0_L2A | 80 m | 43.5409, 103.4944 | 43.6319, 103.3246 |  |
| 43 | S2B_18LUK_20241225_0_L2A | 20 m window | -13.7951, -76.2349 | -13.7981, -76.2287 | Paracas Peninsula (Peru) (9 km) |
| 44 | S2B_28WDT_20231030_0_L2A | 20 m window | 65.5567, -16.9464 | 65.5660, -16.9410 | Myvatn lava and lake, Iceland (5 km) |
| 45 | S2A_19KHQ_20240320_0_L2A | 80 m | -23.0534, -65.9883 | -23.0631, -66.0370 |  |
| 46 | S2B_13SDT_20240527_0_L2A | 20 m window | 33.5769, -106.0708 | 33.5755, -106.0768 | Carrizozo Malpais lava flow, New Mexico USA (18 km) |
| 47 | S2B_26WNE_20240707_0_L2A | 20 m window | 71.4559, -25.6174 | 71.4670, -25.6290 | Scoresby Sund inner fjords, Greenland (6 km) |
| 48 | S2B_42QXM_20240224_0_L2A | 80 m | 24.2114, 70.3645 | 24.0726, 70.4018 |  |
| 49 | S2B_40RFU_20240618_0_L2A | 20 m window | 30.4069, 59.1434 | 30.3837, 59.1355 | Dasht-e Lut kaluts, Iran (10 km) |
| 50 | S2A_33SWB_20241015_0_L2A | 20 m window | 37.8494, 15.0887 | 37.8419, 15.0727 | Mount Etna, Sicily Italy (13 km) |
| 51 | S2B_11TLF_20240316_0_L2A | 80 m | 40.8314, -118.7543 | 40.7758, -118.7541 | Black Rock Desert playa (USA) (30 km) |
| 52 | S2B_44TLP_20241008_0_L2A | 80 m | 43.3907, 78.7549 | 43.4966, 78.8453 | Charyn Canyon, Kazakhstan (25 km) |
| 53 | S2A_38UPV_20240528_0_L2A | 80 m | 49.4840, 47.8819 | 49.3830, 47.8225 |  |
| 54 | S2B_42RXR_20241001_0_L2A | 20 m window | 27.2682, 70.1694 | 27.2593, 70.1625 | Thar Desert (India) (15 km) |
| 55 | S2B_44RNV_20241002_0_L2A | 20 m window | 30.9540, 81.5746 | 30.9623, 81.5768 | Mount Kailash / Lake Manasarovar, Tibet (17 km) |
| 56 | S2A_35KLT_20240122_0_L2A | 20 m window | -20.5883, 26.0678 | -20.5812, 26.0574 | Sua Pan, Makgadikgadi Botswana (4 km) |
| 57 | S2A_47SND_20240330_0_L2A | 80 m | 39.1671, 100.2261 | 39.1532, 100.1759 | Zhangye Danxia, Gansu China (25 km) |
| 58 | S2B_52KDA_20240920_0_L2A | 20 m window | -22.6210, 128.7095 | -22.6186, 128.7148 | Lake Mackay, Australia (13 km) |
| 59 | S2A_11UMT_20240324_0_L2A | 80 m | 52.2857, -117.6990 | 52.2588, -117.7101 |  |
| 60 | S2A_47TQJ_20241006_1_L2A | 20 m window | 43.7456, 102.0349 | 43.6735, 102.0272 | Khongoryn Els dunes, Mongolia (16 km) |
| 61 | S2B_38KNA_20241108_0_L2A | 80 m | -22.4495, 45.9346 | -22.4430, 45.9156 |  |
| 62 | S2A_40QFK_20240911_0_L2A | 80 m | 22.5707, 58.9225 | 22.4962, 58.8872 |  |
| 63 | S2A_27VXL_20241012_0_L2A | 80 m | 63.6544, -17.9904 | 63.6601, -18.1273 | Eldhraun moss-covered lava, Iceland (12 km) |
| 64 | S2A_38UPV_20240925_0_L2A | 80 m | 49.1745, 47.3970 | 48.9990, 47.5977 |  |
| 65 | S2B_26WNE_20240707_0_L2A | 20 m window | 71.6059, -25.8213 | 71.5992, -25.8171 | Scoresby Sund inner fjords, Greenland (16 km) |
| 66 | S2A_11TQF_20240703_0_L2A | 80 m | 41.5014, -114.4960 | 41.5023, -114.4635 |  |
| 67 | S2A_16XDP_20240706_1_L2A | 80 m | 79.3243, -88.4783 | 79.3126, -88.5563 |  |
| 68 | S2B_52KDF_20241030_0_L2A | 80 m | -17.8601, 128.0592 | -17.8385, 128.0592 |  |
| 69 | S2A_41RQJ_20240922_0_L2A | 20 m window | 25.4255, 65.4577 | 25.4199, 65.4621 | Hingol / Makran ranges, Pakistan (9 km) |
| 70 | S2A_33SWB_20241015_0_L2A | 20 m window | 37.7792, 15.1534 | 37.7892, 15.1539 | Mount Etna, Sicily Italy (14 km) |
| 71 | S2B_46RBU_20241109_0_L2A | 20 m window | 30.5282, 90.6598 | 30.5269, 90.6660 | Nam Co, Tibet (20 km) |
| 72 | S2B_34KFD_20241026_0_L2A | 20 m window | -19.2094, 22.8851 | -19.2174, 22.8503 | Okavango Delta, Botswana (10 km) |
| 73 | S2B_18LZH_20241020_0_L2A | 80 m | -15.5313, -72.0576 | -15.5524, -72.0443 | Colca Canyon, Arequipa Peru (17 km) |
| 74 | S2A_48TUP_20241023_0_L2A | 20 m window | 44.0014, 103.6476 | 44.0324, 103.6452 | Flaming Cliffs (Bayanzag), Mongolia (14 km) |
| 75 | S2A_33RYH_20240611_0_L2A | 20 m window | 24.8580, 17.8121 | 24.8480, 17.8218 | Waw an Namus, Libya (9 km) |
| 76 | S2B_40RDQ_20240909_0_L2A | 20 m window | 26.3544, 56.3605 | 26.3600, 56.3785 | Musandam Peninsula, Oman (19 km) |
| 77 | S2B_11SLC_20240604_0_L2A | 80 m | 38.2342, -118.2706 | 38.2403, -118.2009 |  |
| 78 | S2A_36RWS_20240424_0_L2A | 80 m | 28.8869, 33.5641 | 28.8697, 33.5522 |  |
| 79 | S2A_33KVS_20240304_0_L2A | 20 m window | -21.1686, 14.6778 | -21.1732, 14.6709 | Brandberg massif, Namibia (13 km) |
| 80 | S2B_23LMG_20240920_0_L2A | 20 m window | -12.4487, -45.6130 | -12.4537, -45.5874 | Western Bahia centre pivots, Brazil (6 km) |
| 81 | S2B_48TUP_20240521_0_L2A | 20 m window | 44.0855, 103.6303 | 44.0346, 103.6103 | Flaming Cliffs (Bayanzag), Mongolia (11 km) |
| 82 | S2B_47TQJ_20240623_0_L2A | 80 m | 43.7742, 102.5311 | 43.7465, 102.6705 | Khongoryn Els (Mongolia) (25 km) |
| 83 | S2B_38SNG_20240706_0_L2A | 20 m window | 37.3910, 45.5075 | 37.3754, 45.4729 | Lake Urmia, Iran (10 km) |
| 84 | S2B_11SMA_20241009_0_L2A | 20 m window | 36.4978, -117.9908 | 36.5037, -117.9931 | Owens Lake (USA) (10 km) |
| 85 | S2B_11SQR_20241003_0_L2A | 20 m window | 31.7223, -114.7407 | 31.6124, -114.8127 | Colorado River Delta, Gulf of California (4 km) |
| 86 | S2B_46RBT_20241030_0_L2A | 80 m | 29.0265, 90.3209 | 29.0952, 90.3464 |  |
| 87 | S2A_59GMM_20240220_0_L2A | 20 m window | -43.5334, 170.0678 | -43.5057, 169.9939 | Aoraki / Mount Cook, New Zealand (13 km) |
| 88 | S2A_47SND_20240926_0_L2A | 80 m | 39.2030, 99.7017 | 39.0967, 99.5756 |  |
| 89 | S2B_13TGJ_20241004_0_L2A | 20 m window | 43.9087, -102.3145 | 43.8990, -102.2352 | Badlands National Park, South Dakota USA (6 km) |
| 90 | S2B_37PFR_20240417_0_L2A | 20 m window | 13.6861, 40.6154 | 13.7022, 40.6566 | Erta Ale volcano, Ethiopia (11 km) |
| 91 | S2B_46RBU_20241109_0_L2A | 20 m window | 30.5303, 90.5674 | 30.5174, 90.5593 | Nam Co, Tibet (20 km) |
| 92 | S2B_36TXR_20240714_0_L2A | 20 m window | 45.7758, 34.6982 | 45.7977, 34.6551 | Sivash lagoons, Crimea (7 km) |
| 93 | S2B_33XWH_20230705_1_L2A | 80 m | 78.9163, 15.3551 | 78.8919, 15.0206 |  |
| 94 | S2A_33KVQ_20240503_0_L2A | 20 m window | -22.9331, 14.5140 | -22.9425, 14.5150 | Walvis Bay salt works (Namibia) (10 km) |
| 95 | S2B_38KNA_20240422_0_L2A | 20 m window | -22.5964, 45.3391 | -22.5820, 45.3716 | Isalo Massif, Madagascar (4 km) |
| 96 | S2B_32SPD_20240718_0_L2A | 20 m window | 34.7919, 11.2088 | 34.7751, 11.2507 | Kerkennah Islands, Tunisia (10 km) |
| 97 | S2B_11TQJ_20240402_0_L2A | 80 m | 43.8292, -114.2851 | 43.5906, -113.2648 |  |
| 98 | S2B_37PHN_20241130_0_L2A | 80 m | 11.3300, 42.7089 | 10.7912, 42.6576 |  |
| 99 | S2B_37PHN_20241130_0_L2A | 80 m | 11.4780, 42.4693 | 11.5891, 42.5068 | Lake Assal, Djibouti (15 km) |
| 100 | S2B_33PVQ_20240527_0_L2A | 80 m | 13.2945, 14.4940 | 13.1839, 14.6977 |  |
| 101 | S2B_20LNP_20240901_0_L2A | 20 m window | -10.2508, -62.7252 | -10.2711, -62.7182 | Rondonia fishbone deforestation, Brazil (14 km) |
| 102 | S2A_49KGP_20240107_0_L2A | 20 m window | -23.9156, 113.6588 | -23.9225, 113.6634 | Lake MacLeod, Carnarvon (9 km) |
| 103 | S2B_12VVJ_20230829_0_L2A | 20 m window | 56.9991, -111.4252 | 56.9957, -111.4780 | Athabasca oil sands mines, Alberta Canada (6 km) |
| 104 | S2B_37PFR_20240417_0_L2A | 20 m window | 13.5627, 40.6141 | 13.4894, 40.6185 | Erta Ale volcano, Ethiopia (10 km) |
| 105 | S2A_27WWM_20240707_0_L2A | 20 m window | 64.0587, -18.7770 | 64.0580, -18.8012 | Landmannalaugar rhyolite, Iceland (15 km) |
| 106 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.8325, 17.7276 | 24.8606, 17.7228 | Waw an Namus (Libya) (9 km) |
| 107 | S2A_19JDM_20241128_0_L2A | 20 m window | -26.1959, -69.0185 | -26.1991, -69.0485 | Salar de Pedernales, Chile (9 km) |
| 108 | S2B_18LZH_20241020_0_L2A | 80 m | -15.9514, -71.7576 | -15.8471, -71.8190 |  |
| 109 | S2B_12VVJ_20231217_0_L2A | 80 m | 57.6536, -111.1884 | 56.8747, -110.9832 |  |
| 110 | S2A_16QDF_20240205_0_L2A | 20 m window | 18.6579, -87.3826 | 18.6846, -87.3566 | Banco Chinchorro, Mexico (8 km) |
| 111 | S2A_30SWG_20240202_0_L2A | 80 m | 37.8192, -2.3760 | 37.8639, -2.8360 |  |
| 112 | S2A_59GMM_20230605_0_L2A | 20 m window | -44.1250, 170.0934 | -44.1393, 170.0994 | Lake Pukaki, New Zealand (10 km) |
| 113 | S2A_28WDS_20240614_0_L2A | 80 m | 64.4886, -16.9793 | 64.8087, -16.5685 | Holuhraun lava field, Iceland (25 km) |
| 114 | S2A_12SXH_20240617_0_L2A | 80 m | 37.8965, -109.7499 | 38.3981, -109.2309 |  |
| 115 | S2A_38SNG_20241108_0_L2A | 80 m | 37.3432, 45.0819 | 37.3320, 45.1399 |  |
| 116 | S2A_30VVJ_20230226_0_L2A | 20 m window | 56.9433, -3.5527 | 56.9384, -3.5533 | Cairngorms heather moorland, Scotland (13 km) |
| 117 | S2B_18LZH_20241020_0_L2A | 80 m | -15.7144, -71.9463 | -15.8413, -71.8028 | Colca Canyon, Arequipa Peru (20 km) |
| 118 | S2A_32VMP_20240522_0_L2A | 80 m | 61.7676, 7.7329 | 62.0610, 7.1006 |  |
| 119 | S2A_12TWQ_20240411_0_L2A | 20 m window | 44.3420, -110.4952 | 44.3544, -110.4903 | Yellowstone Lake, Wyoming (16 km) |
| 120 | S2B_46TCK_20241003_0_L2A | 20 m window | 40.4137, 90.7197 | 40.4108, 90.7501 | Lop Nur, Xinjiang (9 km) |
| 121 | S2A_57VXC_20240423_0_L2A | 20 m window | 55.9713, 160.7487 | 55.9693, 160.7286 | Klyuchevskaya volcano group, Kamchatka (10 km) |
| 122 | S2B_33XWH_20230705_1_L2A | 20 m window | 78.4893, 16.7773 | 78.4921, 16.7970 | Billefjorden, Svalbard (14 km) |
| 123 | S2B_26WPE_20240717_0_L2A | 80 m | 71.2704, -23.4060 | 71.2495, -23.3684 | Jameson Land, East Greenland (6 km) |
| 124 | S2B_28RFT_20240715_0_L2A | 20 m window | 29.0475, -13.6669 | 29.0512, -13.6729 | Timanfaya, Lanzarote Spain (10 km) |
| 125 | S2B_46TCK_20241003_0_L2A | 20 m window | 40.3989, 90.7942 | 40.3014, 90.8465 | Lop Nur, Xinjiang (2 km) |
| 126 | S2B_52KDA_20240314_0_L2A | 80 m | -22.6630, 128.7026 | -22.6628, 128.6659 | Lake Mackay, Australia (18 km) |
| 127 | S2A_36NZJ_20240704_0_L2A | 80 m | 2.8332, 36.0695 | 2.6972, 35.9661 |  |
| 128 | S2B_12SUG_20241003_0_L2A | 20 m window | 37.1410, -113.1158 | 37.1400, -113.1067 | Zion Canyon, Utah USA (16 km) |
| 129 | S2A_45RVM_20240421_0_L2A | 20 m window | 28.1590, 87.0401 | 28.1343, 87.0215 | Mount Everest / Khumbu, Nepal (20 km) |
| 130 | S2B_32SPD_20240718_0_L2A | 80 m | 34.3100, 10.1640 | 34.3411, 10.1440 |  |
| 131 | S2B_45VXJ_20240129_0_L2A | 20 m window | 61.4340, 89.7746 | 61.4419, 89.7683 | Yenisei taiga, Russia (14 km) |
| 132 | S2B_28RFT_20240715_0_L2A | 80 m | 28.9292, -13.8109 | 28.9181, -13.8423 | Timanfaya, Lanzarote Spain (11 km) |
| 133 | S2B_50JKP_20240418_0_L2A | 80 m | -28.4693, 114.5392 | -28.5076, 114.5218 |  |
| 134 | S2A_38SNG_20241108_0_L2A | 20 m window | 37.2951, 45.5627 | 37.3008, 45.5328 | Lake Urmia, Iran (12 km) |
| 135 | S2B_26WPE_20240717_0_L2A | 20 m window | 71.3191, -23.6090 | 71.2970, -23.5424 | Jameson Land, East Greenland (3 km) |
| 136 | S2B_52KDA_20240314_0_L2A | 80 m | -22.5561, 128.7323 | -22.5905, 128.7280 | Lake Mackay, Australia (9 km) |
| 137 | S2A_30TXM_20240123_0_L2A | 20 m window | 42.1123, -1.6175 | 42.1042, -1.6275 | Bardenas Reales, Navarre Spain (14 km) |
| 138 | S2B_32TMS_20241231_0_L2A | 80 m | 46.0847, 8.4055 | 46.0827, 8.4395 |  |
| 139 | S2A_16XDP_20240706_1_L2A | 20 m window | 79.3919, -89.8337 | 79.3961, -89.8086 | Axel Heiberg Island, Nunavut Canada (18 km) |
| 140 | S2B_18GXP_20230619_0_L2A | 80 m | -46.7960, -72.9004 | -46.7636, -72.8751 | Lago General Carrera, Chile (25 km) |
| 141 | S2B_13SCS_20240527_0_L2A | 80 m | 33.1993, -106.6656 | 33.3445, -106.7148 |  |
| 142 | S2A_19KFQ_20240919_0_L2A | 80 m | -22.7905, -67.7545 | -22.8217, -67.6841 | Laguna Verde / Licancabur, Bolivia (11 km) |
| 143 | S2A_44SKA_20240930_0_L2A | 20 m window | 32.2303, 78.0567 | 32.2051, 78.0710 | Spiti Valley, India (4 km) |
| 144 | S2B_12VVJ_20231217_0_L2A | 80 m | 57.5922, -112.1062 | 57.5710, -112.1035 |  |
| 145 | S2B_12VVJ_20231217_0_L2A | 20 m window | 56.9261, -111.5410 | 56.9197, -111.5414 | Athabasca oil sands mines, Alberta Canada (9 km) |
| 146 | S2A_11UMT_20240324_0_L2A | 80 m | 52.2433, -118.3566 | 52.2446, -118.3830 |  |
| 147 | S2A_11UMT_20240324_0_L2A | 20 m window | 52.1973, -117.4654 | 52.1442, -117.3398 | Columbia Icefield, Alberta/BC Canada (7 km) |
| 148 | S2A_44SKA_20240930_0_L2A | 80 m | 31.9039, 78.0916 | 31.8099, 78.0797 |  |
| 149 | S2B_12SVG_20240605_0_L2A | 20 m window | 37.0968, -111.0465 | 37.0959, -111.0403 | Lake Powell / Navajo Mountain, Utah USA (8 km) |
| 150 | S2B_11TLF_20240316_0_L2A | 20 m window | 40.7383, -119.1346 | 40.7473, -119.1310 | Black Rock Desert playa (USA) (12 km) |
| 151 | S2A_22WEV_20230311_0_L2A | 80 m | 67.0611, -50.2314 | 67.0617, -50.1850 | Kangerlussuaq ice margin, Greenland (6 km) |
| 152 | S2B_38KNA_20241108_0_L2A | 80 m | -21.9359, 45.3481 | -21.9642, 45.3465 |  |
| 153 | S2B_38UPU_20240907_0_L2A | 80 m | 47.8711, 47.6903 | 47.8919, 47.6873 |  |
| 154 | S2B_11TLF_20240316_0_L2A | 80 m | 40.9711, -118.7454 | 41.0894, -118.6447 |  |
| 155 | S2A_44SKA_20240513_0_L2A | 80 m | 32.2749, 78.0595 | 32.2909, 78.0672 | Spiti Valley, India (4 km) |
| 156 | S2A_20KKU_20240718_0_L2A | 80 m | -23.9968, -65.5813 | -23.9789, -65.5684 |  |
| 157 | S2A_33XWG_20240718_2_L2A | 20 m window | 78.1979, 17.0920 | 78.1967, 17.1110 | Nordenskiold Land, Svalbard Norway (13 km) |
| 158 | S2B_40RFU_20241009_0_L2A | 20 m window | 30.3016, 58.4149 | 30.2957, 58.4304 | Kaluts, Dasht-e Lut (Iran) (11 km) |
| 159 | S2A_19JFM_20240110_0_L2A | 80 m | -25.5999, -67.7424 | -25.5214, -67.5772 | Salar de Antofalla, Argentina (6 km) |
| 160 | S2B_12SWJ_20240705_0_L2A | 80 m | 39.5281, -110.9108 | 39.5772, -110.9454 |  |
| 161 | S2B_26WPE_20240717_0_L2A | 20 m window | 71.2503, -23.7154 | 71.2481, -23.6772 | Jameson Land, East Greenland (9 km) |
| 162 | S2A_44RNV_20240417_0_L2A | 80 m | 30.8944, 81.6564 | 30.9101, 81.6525 | Mount Kailash / Lake Manasarovar, Tibet (20 km) |
| 163 | S2A_59GMM_20230605_0_L2A | 80 m | -43.5170, 170.8398 | -43.5016, 170.7919 |  |
| 164 | S2B_37RCK_20240602_0_L2A | 80 m | 26.8776, 37.6323 | 26.8546, 37.6295 |  |
| 165 | S2B_51XXA_20240710_0_L2A | 20 m window | 72.7354, 126.4205 | 72.7288, 126.4136 | Lena Delta, Russia (8 km) |
| 166 | S2B_19KFP_20240407_0_L2A | 80 m | -24.1996, -67.9726 | -24.1800, -67.8874 |  |
| 167 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.9242, 17.7381 | 24.8716, 17.6452 | Waw an Namus (Libya) (6 km) |
| 168 | S2A_35KLT_20240730_0_L2A | 80 m | -19.9427, 25.2728 | -20.2673, 25.4265 |  |
| 169 | S2B_37PHN_20241130_0_L2A | 20 m window | 11.5215, 42.4791 | 11.5181, 42.4941 | Lake Assal, Djibouti (16 km) |
| 170 | S2A_38SNG_20241108_0_L2A | 80 m | 37.4215, 45.7399 | 37.0241, 45.7588 | Lake Urmia, Iran (24 km) |
| 171 | S2B_54HXF_20240422_0_L2A | 80 m | -36.0484, 142.4149 | -36.0079, 142.3933 |  |
| 172 | S2B_46RBU_20241109_0_L2A | 20 m window | 30.5360, 90.6040 | 30.5291, 90.6012 | Nam Co, Tibet (19 km) |
| 173 | S2B_19KFR_20241004_0_L2A | 80 m | -22.0047, -67.5787 | -22.0090, -67.5095 |  |
| 174 | S2B_44TLP_20240421_0_L2A | 20 m window | 43.3055, 79.0645 | 43.3051, 79.0816 | Charyn Canyon, Kazakhstan (5 km) |
| 175 | S2B_46TCK_20241003_0_L2A | 80 m | 40.2712, 91.5841 | 40.2039, 91.7769 |  |
| 176 | S2B_52KDA_20240314_0_L2A | 20 m window | -22.4005, 128.7017 | -22.4115, 128.6983 | Lake Mackay, Australia (10 km) |
| 177 | S2A_12SXD_20241005_0_L2A | 20 m window | 35.1483, -109.8025 | 35.1593, -109.8035 | Painted Desert / Petrified Forest, Arizona USA (10 km) |
| 178 | S2B_57VXC_20241015_0_L2A | 20 m window | 56.0800, 160.8925 | 56.0788, 160.9433 | Klyuchevskaya volcano group, Kamchatka (17 km) |
| 179 | S2A_43SFV_20240923_0_L2A | 80 m | 35.1648, 76.3418 | 35.2666, 76.7952 |  |
| 180 | S2A_57VXC_20240423_0_L2A | 80 m | 55.9806, 160.6969 | 55.9552, 160.7153 | Klyuchevskaya volcano group, Kamchatka (10 km) |
| 181 | S2A_22WEV_20240812_0_L2A | 20 m window | 67.2146, -50.1521 | 67.2117, -50.1246 | Greenland ice margin, Kangerlussuaq (11 km) |
| 182 | S2B_26WNE_20240707_0_L2A | 20 m window | 71.5725, -25.8736 | 71.5910, -25.8093 | Scoresby Sund inner fjords, Greenland (15 km) |
| 183 | S2A_47UPS_20240402_0_L2A | 20 m window | 50.9655, 100.5422 | 50.9672, 100.5040 | Lake Khovsgol, Mongolia (4 km) |
| 184 | S2B_11TQJ_20240402_0_L2A | 80 m | 43.3680, -113.5305 | 43.3523, -113.5613 | Craters of the Moon lava field, Idaho USA (1 km) |
| 185 | S2A_40RDR_20240527_0_L2A | 20 m window | 27.1599, 56.5220 | 27.1575, 56.5270 | Hormuz Island, Iran (13 km) |
| 186 | S2B_26WNE_20240707_0_L2A | 80 m | 72.0243, -25.9288 | 72.0140, -25.9114 |  |
| 187 | S2A_51KWB_20240613_0_L2A | 20 m window | -16.5049, 123.4356 | -16.5064, 123.4445 | Buccaneer Archipelago, Kimberley Australia (23 km) |
| 188 | S2B_12VVJ_20231217_0_L2A | 20 m window | 57.0507, -111.4244 | 57.0548, -111.4248 | Athabasca oil sands mines, Alberta Canada (10 km) |
| 189 | S2B_52KDA_20240314_0_L2A | 20 m window | -22.4202, 128.8065 | -22.3988, 128.8021 | Lake Mackay, Australia (15 km) |
| 190 | S2B_34QBG_20241011_0_L2A | 20 m window | 19.8192, 18.5306 | 19.8273, 18.5170 | Emi Koussi, Tibesti Chad (5 km) |
| 191 | S2A_13RGK_20240327_0_L2A | 80 m | 26.4966, -102.5804 | 26.5186, -102.6018 |  |
| 192 | S2A_16XDP_20240706_1_L2A | 80 m | 79.6955, -87.1546 | 79.5589, -87.3157 |  |
| 193 | S2B_21KVA_20240922_0_L2A | 80 m | -17.3907, -57.9276 | -17.4019, -57.9380 |  |
| 194 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.8500, 17.7580 | 24.8342, 17.7555 | Waw an Namus (Libya) (9 km) |
| 195 | S2A_40RDR_20240527_0_L2A | 20 m window | 27.1740, 56.4085 | 27.1712, 56.3853 | Hormuz Island, Iran (14 km) |
| 196 | S2B_40RFU_20240621_0_L2A | 20 m window | 30.4360, 58.3951 | 30.4386, 58.3858 | Kaluts, Dasht-e Lut (Iran) (4 km) |
| 197 | S2A_43TEM_20240621_0_L2A | 80 m | 46.3399, 75.1071 | 46.3477, 75.1423 | Lake Balkhash west, Kazakhstan (29 km) |
| 198 | S2B_19GBR_20240517_0_L2A | 80 m | -40.2859, -71.7504 | -40.2667, -71.7916 |  |
| 199 | S2B_39QXG_20240912_0_L2A | 80 m | 24.1353, 52.7184 | 24.1867, 52.5952 | Al Gharbia sabkha coast, UAE (16 km) |
| 200 | S2A_36MYB_20230210_0_L2A | 20 m window | -3.0931, 35.4968 | -3.0892, 35.4965 | Ngorongoro Crater, Tanzania (14 km) |
| 201 | S2A_45RVM_20241028_0_L2A | 20 m window | 27.9705, 86.8680 | 27.9747, 86.8693 | Everest massif (Nepal/China) (4 km) |
| 202 | S2B_32MNC_20240818_0_L2A | 80 m | -2.6337, 9.6827 | -2.6137, 9.6941 |  |
| 203 | S2B_45QYE_20241116_0_L2A | 80 m | 21.8993, 89.8495 | 21.8918, 89.8324 |  |
| 204 | S2A_33SWB_20241015_0_L2A | 20 m window | 37.7003, 15.0686 | 37.6942, 15.0705 | Mount Etna, Sicily Italy (8 km) |
| 205 | S2A_12TWQ_20240411_0_L2A | 80 m | 44.9228, -110.3597 | 44.9207, -110.3289 |  |
| 206 | S2B_52KDA_20240314_0_L2A | 20 m window | -22.4211, 128.7981 | -22.4357, 128.7911 | Lake Mackay, Australia (13 km) |
| 207 | S2A_28WDT_20240919_0_L2A | 80 m | 65.0228, -16.4691 | 65.1072, -16.4059 | Askja and Holuhraun lava field, Iceland (16 km) |
| 208 | S2A_47UPS_20240402_0_L2A | 80 m | 50.8207, 100.4875 | 50.8347, 100.4338 | Lake Khovsgol, Mongolia (19 km) |
| 209 | S2B_32TNK_20241228_0_L2A | 20 m window | 39.9806, 9.3303 | 39.9702, 9.3353 | Central Sardinian maquis, Italy (4 km) |
| 210 | S2B_57VXC_20241015_0_L2A | 20 m window | 56.0019, 160.9283 | 56.0070, 160.9260 | Klyuchevskaya volcano group, Kamchatka (18 km) |
| 211 | S2B_11SLC_20240604_0_L2A | 80 m | 38.3090, -118.8659 | 38.0022, -118.8127 | Mono Lake, California (22 km) |
| 212 | S2B_11SPS_20240229_0_L2A | 80 m | 33.2389, -115.5910 | 33.2195, -115.5687 | Salton Sea, California (22 km) |
| 213 | S2B_12VVJ_20230829_0_L2A | 80 m | 57.3140, -110.9963 | 57.5633, -110.9866 |  |
| 214 | S2B_38KNA_20241108_0_L2A | 20 m window | -22.6414, 45.2339 | -22.6397, 45.2296 | Isalo Massif, Madagascar (16 km) |
| 215 | S2B_28WDT_20231030_0_L2A | 20 m window | 65.5245, -17.0378 | 65.5277, -17.0304 | Myvatn lava and lake, Iceland (8 km) |
| 216 | S2A_35KLT_20240122_0_L2A | 80 m | -20.6754, 25.9059 | -20.7859, 25.8900 | Sua Pan, Makgadikgadi, Botswana (15 km) |
| 217 | S2B_11TLF_20240316_0_L2A | 20 m window | 40.8146, -119.2378 | 40.8246, -119.2568 | Black Rock Desert playa (USA) (13 km) |
| 218 | S2B_51XXA_20240710_0_L2A | 20 m window | 72.6916, 126.8791 | 72.7106, 126.8457 | Lena Delta, Russia (16 km) |
| 219 | S2A_30RUV_20241009_0_L2A | 80 m | 31.4904, -4.3906 | 31.5398, -4.4053 |  |
| 220 | S2B_28RFT_20240715_0_L2A | 20 m window | 28.8879, -13.7457 | 28.8953, -13.8026 | Timanfaya, Lanzarote Spain (12 km) |
| 221 | S2B_38PKT_20240603_0_L2A | 20 m window | 11.6374, 42.4597 | 11.6339, 42.4612 | Lake Assal, Djibouti (5 km) |
| 222 | S2A_18WXD_20240818_0_L2A | 20 m window | 70.6845, -71.5756 | 70.7058, -71.4810 | Sam Ford Fjord, Baffin Island (12 km) |
| 223 | S2B_26WNE_20240707_0_L2A | 80 m | 71.5801, -24.9593 | 71.5668, -24.9977 | Scoresby Sund inner fjords, Greenland (20 km) |
| 224 | S2B_12SVG_20241003_0_L2A | 20 m window | 37.0196, -110.9268 | 37.0228, -110.9231 | Lake Powell / Navajo Mountain, Utah USA (9 km) |
| 225 | S2B_13SDT_20240904_0_L2A | 20 m window | 33.6062, -106.0041 | 33.6338, -105.9551 | Carrizozo Malpais lava flow, New Mexico USA (9 km) |
| 226 | S2A_27WWM_20240707_0_L2A | 20 m window | 64.0046, -19.2219 | 64.0028, -19.2050 | Landmannalaugar (Iceland) (7 km) |
| 227 | S2A_43TDM_20241009_0_L2A | 80 m | 46.8695, 74.8497 | 46.8732, 74.8023 |  |
| 228 | S2B_11TLF_20240316_0_L2A | 80 m | 40.8442, -118.3261 | 40.8814, -118.2839 |  |
| 229 | S2A_50JKP_20241209_0_L2A | 80 m | -28.0176, 114.4937 | -28.0058, 114.4433 | Hutt Lagoon (Australia) (26 km) |
| 230 | S2B_52JFT_20240111_0_L2A | 80 m | -24.8423, 130.9604 | -24.8515, 130.9369 |  |
| 231 | S2B_38QNL_20240603_0_L2A | 80 m | 22.7854, 45.0760 | 22.7909, 45.0936 |  |
| 232 | S2B_12SVG_20240605_0_L2A | 20 m window | 37.1260, -110.9845 | 37.1213, -110.9862 | Lake Powell / Navajo Mountain, Utah USA (4 km) |
| 233 | S2B_40RDQ_20240909_0_L2A | 80 m | 27.0046, 56.9804 | 26.9833, 56.9614 |  |
| 234 | S2A_35KLT_20240122_0_L2A | 80 m | -20.4897, 25.6193 | -20.4660, 25.6204 | Makgadikgadi Pans (Botswana) (19 km) |
| 235 | S2A_32VMP_20240522_0_L2A | 80 m | 61.8058, 7.7586 | 61.8059, 7.7238 |  |
| 236 | S2B_46RBT_20241030_0_L2A | 80 m | 29.5255, 90.5476 | 29.4967, 90.5611 |  |
| 237 | S2A_40QEL_20240524_0_L2A | 80 m | 22.9594, 57.7741 | 22.9387, 57.7929 |  |
| 238 | S2B_50JKP_20240418_0_L2A | 20 m window | -28.1071, 114.1842 | -28.1026, 114.1865 | Hutt Lagoon, Western Australia (7 km) |
| 239 | S2A_33XWG_20240718_2_L2A | 20 m window | 78.2509, 16.5145 | 78.2426, 16.5012 | Nordenskiold Land, Svalbard Norway (9 km) |
| 240 | S2A_12SWJ_20240211_0_L2A | 20 m window | 38.9025, -110.7831 | 38.9089, -110.7811 | San Rafael Swell, Utah USA (7 km) |
| 241 | S2B_57VXC_20241015_0_L2A | 20 m window | 56.0012, 160.9497 | 56.0039, 160.9240 | Klyuchevskaya volcano group, Kamchatka (19 km) |
| 242 | S2A_13RGK_20240327_0_L2A | 20 m window | 26.7458, -102.0463 | 26.7975, -102.1421 | Cuatro Cienegas gypsum dunes (Mexico) (9 km) |
| 243 | S2B_40TEQ_20240418_0_L2A | 20 m window | 44.8835, 58.0514 | 44.8903, 58.0461 | Aral Sea west basin, Aralkum (27 km) |
| 244 | S2A_43TEM_20240621_0_L2A | 80 m | 46.8402, 76.0529 | 46.8624, 76.0657 |  |
| 245 | S2A_11SMA_20240616_0_L2A | 80 m | 36.4785, -117.4285 | 36.9035, -117.5331 |  |
| 246 | S2B_13TGG_20240606_0_L2A | 80 m | 42.2156, -101.6893 | 42.1863, -101.7162 | Nebraska Sand Hills, USA (27 km) |
| 247 | S2A_33SWB_20241015_0_L2A | 80 m | 37.1612, 15.0798 | 37.1109, 15.1245 |  |
| 248 | S2B_52KDF_20241030_0_L2A | 80 m | -17.9295, 128.7970 | -17.7389, 128.7727 |  |
| 249 | S2B_19KFR_20240308_0_L2A | 20 m window | -22.2049, -67.7213 | -22.1868, -67.7078 | Laguna Colorada, Bolivia (7 km) |
| 250 | S2A_37REJ_20241005_0_L2A | 80 m | 25.2820, 39.7790 | 25.3667, 39.8279 |  |
| 251 | S2A_50KNV_20240321_0_L2A | 20 m window | -22.7846, 117.8609 | -22.6813, 117.9076 | Hamersley Range (Australia) (9 km) |
| 252 | S2B_5VLE_20240615_0_L2A | 80 m | 58.1244, -155.0226 | 58.0862, -155.0054 |  |
| 253 | S2B_52KDA_20240920_0_L2A | 20 m window | -22.5965, 128.7843 | -22.6041, 128.7748 | Lake Mackay, Australia (14 km) |
| 254 | S2B_34KFD_20241026_0_L2A | 80 m | -19.2619, 22.3733 | -19.2540, 22.3908 |  |
| 255 | S2A_57VXC_20240423_0_L2A | 80 m | 56.4991, 161.6285 | 56.5056, 161.4992 |  |
| 256 | S2B_12SUE_20240615_0_L2A | 80 m | 35.9965, -112.4684 | 36.0638, -112.4435 |  |
| 257 | S2A_12SXH_20240617_0_L2A | 80 m | 38.0087, -108.6214 | 37.9926, -108.6621 |  |
| 258 | S2A_11TQF_20240703_0_L2A | 80 m | 41.4567, -114.2927 | 41.4711, -114.2402 |  |
| 259 | S2A_19JDM_20240323_0_L2A | 20 m window | -26.1035, -69.2321 | -26.1008, -69.2024 | Salar de Pedernales, Chile (20 km) |
| 260 | S2B_44TLP_20240421_0_L2A | 20 m window | 43.4511, 78.9731 | 43.4684, 78.9906 | Charyn Canyon, Kazakhstan (15 km) |
| 261 | S2A_19KEQ_20240103_0_L2A | 80 m | -23.3386, -68.4603 | -23.3900, -68.4972 | Salar de Atacama lithium ponds, Chile (26 km) |
| 262 | S2B_19KFR_20241004_0_L2A | 80 m | -21.9247, -67.8858 | -21.8926, -67.8851 |  |
| 263 | S2B_40REU_20241009_0_L2A | 20 m window | 30.2914, 57.9930 | 30.3276, 57.9849 | Dasht-e Lut yardangs, Iran (10 km) |
| 264 | S2B_48TUP_20240521_0_L2A | 80 m | 43.9233, 102.6724 | 43.9836, 102.8338 |  |
| 265 | S2A_12STA_20241008_0_L2A | 80 m | 31.7613, -113.0703 | 31.7997, -113.0713 |  |
| 266 | S2B_19KFP_20240407_0_L2A | 80 m | -23.7825, -67.2629 | -23.8358, -67.2990 |  |
| 267 | S2B_51KWT_20241026_0_L2A | 80 m | -20.1973, 123.0818 | -20.1825, 123.0372 |  |
| 268 | S2A_33JYK_20241203_0_L2A | 80 m | -28.0142, 17.4002 | -27.9947, 17.4006 |  |
| 269 | S2B_19KFR_20241004_0_L2A | 80 m | -22.0364, -68.0202 | -21.8157, -67.9637 |  |
| 270 | S2B_47TQJ_20240623_0_L2A | 20 m window | 43.8800, 102.3338 | 43.8805, 102.3567 | Khongoryn Els (Mongolia) (10 km) |
| 271 | S2A_19KEQ_20240103_0_L2A | 80 m | -23.2465, -68.0794 | -23.3915, -68.0430 | Salar de Atacama, Chile (28 km) |
| 272 | S2B_14SKG_20240613_0_L2A | 20 m window | 37.6527, -101.2661 | 37.6591, -101.2300 | Southwest Kansas centre pivots, USA (4 km) |
| 273 | S2A_32QMF_20241008_0_L2A | 80 m | 18.8241, 8.2315 | 18.8895, 8.2127 |  |
| 274 | S2A_33KVQ_20240503_0_L2A | 20 m window | -23.1221, 14.4781 | -23.1122, 14.4641 | Walvis Bay salt works and Sandwich Harbour (4 km) |
| 275 | S2B_19LDC_20240808_0_L2A | 80 m | -16.0331, -69.6933 | -16.2951, -69.6528 |  |
| 276 | S2A_30SWF_20230415_0_L2A | 20 m window | 36.8258, -2.9462 | 36.8191, -2.9439 | El Ejido greenhouses (Spain) (15 km) |
| 277 | S2B_44TLP_20240421_0_L2A | 20 m window | 43.3460, 79.0919 | 43.3302, 79.1193 | Charyn Canyon, Kazakhstan (2 km) |
| 278 | S2A_20KKU_20241115_0_L2A | 80 m | -23.6100, -65.4176 | -23.6316, -65.4621 |  |
| 279 | S2B_29RPP_20240609_0_L2A | 20 m window | 29.9455, -7.4256 | 29.9518, -7.4195 | Anti-Atlas folds, Morocco (9 km) |
| 280 | S2B_33KVS_20241114_0_L2A | 80 m | -21.3635, 14.4477 | -21.4266, 14.4625 | Brandberg massif, Namibia (29 km) |
| 281 | S2A_44RNV_20240417_0_L2A | 20 m window | 30.8834, 81.3502 | 30.8867, 81.3569 | Mount Kailash / Lake Manasarovar, Tibet (10 km) |
| 282 | S2A_35QLF_20240910_0_L2A | 20 m window | 23.4718, 25.9351 | 23.4774, 25.9245 | Gilf Kebir, Egypt (8 km) |
| 283 | S2A_49KGP_20240107_0_L2A | 20 m window | -23.9399, 113.7620 | -23.9440, 113.7521 | Lake MacLeod, Carnarvon (13 km) |
| 284 | S2A_11SMA_20240616_0_L2A | 80 m | 36.6302, -117.8019 | 36.6664, -117.8194 | Owens Lake (USA) (28 km) |
| 285 | S2A_41RQJ_20240922_0_L2A | 80 m | 25.8355, 66.0077 | 25.8306, 65.9653 |  |
| 286 | S2B_11TLF_20240316_0_L2A | 80 m | 40.8453, -118.3015 | 40.7252, -118.3752 |  |
| 287 | S2B_33JYJ_20240402_0_L2A | 20 m window | -28.2432, 17.1378 | -28.2477, 17.1324 | Richtersveld, South Africa (13 km) |
| 288 | S2B_19KEP_20240427_0_L2A | 80 m | -23.6007, -68.0538 | -23.6407, -67.9588 | Lagunas Miscanti and Minique, Chile (27 km) |
| 289 | S2B_11SNA_20241009_0_L2A | 20 m window | 36.1485, -116.7088 | 36.1540, -116.7076 | Badwater Basin, Death Valley (10 km) |
| 290 | S2A_19JFM_20240519_0_L2A | 20 m window | -25.6896, -67.6912 | -25.6616, -67.6887 | Salar de Antofalla, Argentina (9 km) |
| 291 | S2B_36MZD_20240805_0_L2A | 20 m window | -1.8219, 36.2601 | -1.8266, 36.2598 | Lake Magadi, Kenya (5 km) |
| 292 | S2B_12VVJ_20230829_0_L2A | 20 m window | 56.9522, -111.3760 | 56.9455, -111.3794 | Athabasca oil sands mines, Alberta Canada (12 km) |
| 293 | S2B_14SKG_20240613_0_L2A | 20 m window | 37.5582, -101.4257 | 37.5627, -101.4286 | Southwest Kansas centre pivots, USA (22 km) |
| 294 | S2A_32KRE_20240824_0_L2A | 80 m | -18.1769, 12.7347 | -18.1625, 12.7486 |  |
| 295 | S2A_36NZJ_20240126_0_L2A | 80 m | 3.4272, 36.5015 | 3.4176, 36.5572 |  |
| 296 | S2A_36NZJ_20240704_0_L2A | 80 m | 3.2464, 36.3925 | 3.2442, 36.3634 |  |
| 297 | S2A_36MZC_20230128_0_L2A | 80 m | -2.6398, 35.7129 | -2.6818, 35.8120 | Ol Doinyo Lengai, Tanzania (20 km) |
| 298 | S2A_14SKG_20241006_0_L2A | 20 m window | 37.7537, -101.3594 | 37.7595, -101.3545 | Southwest Kansas centre pivots, USA (18 km) |
| 299 | S2B_11SQR_20241003_0_L2A | 20 m window | 31.7955, -114.7295 | 31.8147, -114.7675 | Colorado River delta, Mexico (6 km) |
| 300 | S2A_37REJ_20241005_0_L2A | 20 m window | 25.8161, 39.7985 | 25.8151, 39.8038 | Harrat Khaybar, Saudi Arabia (10 km) |
| 301 | S2A_53KKP_20240412_0_L2A | 80 m | -24.1441, 132.4829 | -24.0794, 132.6030 |  |
| 302 | S2B_14SKG_20240613_0_L2A | 20 m window | 37.5550, -101.2780 | 37.5653, -101.2878 | Southwest Kansas centre pivots, USA (12 km) |
| 303 | S2A_19JFM_20240519_0_L2A | 20 m window | -25.5863, -67.5062 | -25.5882, -67.5187 | Salar de Antofalla, Argentina (10 km) |
| 304 | S2A_53HLE_20240110_0_L2A | 20 m window | -31.9321, 132.9611 | -31.9253, 132.9609 | Lake MacDonnell, South Australia (11 km) |
| 305 | S2B_48TUP_20240521_0_L2A | 80 m | 43.5352, 103.5011 | 43.7092, 103.2683 |  |
| 306 | S2B_12SVH_20241003_0_L2A | 20 m window | 38.2203, -111.2653 | 38.2231, -111.2701 | Capitol Reef / Waterpocket Fold, Utah USA (17 km) |
| 307 | S2B_11SNA_20241009_0_L2A | 80 m | 36.8380, -116.5718 | 36.7809, -116.5890 |  |
| 308 | S2A_36SXH_20241004_0_L2A | 80 m | 38.2542, 35.0264 | 38.2740, 35.0791 |  |
| 309 | S2B_37RCK_20240602_0_L2A | 20 m window | 26.6084, 37.7800 | 26.5999, 37.7811 | AlUla sandstone (Saudi Arabia) (12 km) |
| 310 | S2A_41RQJ_20240515_0_L2A | 20 m window | 25.5552, 65.5737 | 25.5450, 65.5290 | Hingol / Makran ranges, Pakistan (8 km) |
| 311 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.8850, 17.7073 | 24.8209, 17.6760 | Waw an Namus (Libya) (9 km) |
| 312 | S2B_44TLP_20241008_0_L2A | 20 m window | 44.0993, 78.9879 | 44.0884, 78.9823 | Aktau Mountains, Altyn-Emel Kazakhstan (12 km) |
| 313 | S2A_45TYH_20240623_0_L2A | 80 m | 43.0714, 90.3846 | 42.9501, 90.0631 |  |
| 314 | S2B_57VXC_20241015_0_L2A | 20 m window | 56.0012, 160.9497 | 56.0039, 160.9240 | Klyuchevskaya volcano group, Kamchatka (19 km) |
| 315 | S2A_43TEM_20240621_0_L2A | 80 m | 46.8402, 76.0529 | 46.8624, 76.0657 |  |
| 316 | S2B_19KFR_20240308_0_L2A | 20 m window | -22.2049, -67.7213 | -22.1868, -67.7078 | Laguna Colorada, Bolivia (7 km) |
| 317 | S2A_37REJ_20241005_0_L2A | 80 m | 25.2820, 39.7790 | 25.3667, 39.8279 |  |
| 318 | S2B_5VLE_20240615_0_L2A | 80 m | 58.1244, -155.0226 | 58.0862, -155.0054 |  |
| 319 | S2B_52KDA_20240920_0_L2A | 20 m window | -22.5965, 128.7843 | -22.6041, 128.7748 | Lake Mackay, Australia (14 km) |
| 320 | S2B_34KFD_20241026_0_L2A | 80 m | -19.2619, 22.3733 | -19.2540, 22.3908 |  |
| 321 | S2A_57VXC_20240423_0_L2A | 80 m | 56.4991, 161.6285 | 56.5056, 161.4992 |  |
| 322 | S2B_12SUE_20240615_0_L2A | 80 m | 35.9965, -112.4684 | 36.0638, -112.4435 |  |
| 323 | S2A_12SXH_20240617_0_L2A | 80 m | 38.0087, -108.6214 | 37.9926, -108.6621 |  |
| 324 | S2A_11TQF_20240703_0_L2A | 80 m | 41.4567, -114.2927 | 41.4711, -114.2402 |  |
| 325 | S2A_19JDM_20240323_0_L2A | 20 m window | -26.1035, -69.2321 | -26.1008, -69.2024 | Salar de Pedernales, Chile (20 km) |
| 326 | S2B_48PXR_20230313_0_L2A | 20 m window | 9.7149, 106.5521 | 9.7202, 106.5498 | Mekong River mouths, Vietnam (14 km) |
| 327 | S2B_44TLP_20240809_0_L2A | 80 m | 43.2641, 78.8996 | 43.2834, 78.7484 | Charyn Canyon, Kazakhstan (22 km) |
| 328 | S2B_44TLP_20240421_0_L2A | 20 m window | 43.4511, 78.9731 | 43.4684, 78.9906 | Charyn Canyon, Kazakhstan (15 km) |
| 329 | S2A_19KEQ_20240103_0_L2A | 80 m | -23.3386, -68.4603 | -23.3900, -68.4972 | Salar de Atacama lithium ponds, Chile (26 km) |
| 330 | S2B_19KFR_20241004_0_L2A | 80 m | -21.9247, -67.8858 | -21.8926, -67.8851 |  |
| 331 | S2B_40REU_20241009_0_L2A | 20 m window | 30.2914, 57.9930 | 30.3276, 57.9849 | Dasht-e Lut yardangs, Iran (10 km) |
| 332 | S2B_48TUP_20240521_0_L2A | 80 m | 43.9233, 102.6724 | 43.9836, 102.8338 |  |
| 333 | S2A_12STA_20241008_0_L2A | 80 m | 31.7613, -113.0703 | 31.7997, -113.0713 |  |
| 334 | S2B_19KFP_20240407_0_L2A | 80 m | -23.7825, -67.2629 | -23.8358, -67.2990 |  |
| 335 | S2B_51KWT_20241026_0_L2A | 80 m | -20.1973, 123.0818 | -20.1825, 123.0372 |  |
| 336 | S2A_33JYK_20241203_0_L2A | 80 m | -28.0142, 17.4002 | -27.9947, 17.4006 |  |
| 337 | S2B_19KFR_20241004_0_L2A | 80 m | -22.0364, -68.0202 | -21.8157, -67.9637 |  |
| 338 | S2B_47TQJ_20240623_0_L2A | 20 m window | 43.8800, 102.3338 | 43.8805, 102.3567 | Khongoryn Els (Mongolia) (10 km) |
| 339 | S2A_19KEQ_20240103_0_L2A | 80 m | -23.2465, -68.0794 | -23.3915, -68.0430 | Salar de Atacama, Chile (28 km) |
| 340 | S2B_14SKG_20240613_0_L2A | 20 m window | 37.6527, -101.2661 | 37.6591, -101.2300 | Southwest Kansas centre pivots, USA (4 km) |
| 341 | S2A_32QMF_20241008_0_L2A | 80 m | 18.8241, 8.2315 | 18.8895, 8.2127 |  |
| 342 | S2A_33KVQ_20240503_0_L2A | 20 m window | -23.1221, 14.4781 | -23.1122, 14.4641 | Walvis Bay salt works and Sandwich Harbour (4 km) |
| 343 | S2B_19LDC_20240808_0_L2A | 80 m | -16.0331, -69.6933 | -16.2951, -69.6528 |  |
| 344 | S2A_30SWF_20230415_0_L2A | 20 m window | 36.8258, -2.9462 | 36.8191, -2.9439 | El Ejido greenhouses (Spain) (15 km) |
| 345 | S2B_44TLP_20240421_0_L2A | 20 m window | 43.3460, 79.0919 | 43.3302, 79.1193 | Charyn Canyon, Kazakhstan (2 km) |
| 346 | S2A_20KKU_20241115_0_L2A | 80 m | -23.6100, -65.4176 | -23.6316, -65.4621 |  |
| 347 | S2B_29RPP_20240609_0_L2A | 20 m window | 29.9455, -7.4256 | 29.9518, -7.4195 | Anti-Atlas folds, Morocco (9 km) |
| 348 | S2B_33KVS_20241114_0_L2A | 80 m | -21.3635, 14.4477 | -21.4266, 14.4625 | Brandberg massif, Namibia (29 km) |
| 349 | S2A_44RNV_20240417_0_L2A | 20 m window | 30.8834, 81.3502 | 30.8867, 81.3569 | Mount Kailash / Lake Manasarovar, Tibet (10 km) |
| 350 | S2A_35QLF_20240910_0_L2A | 20 m window | 23.4718, 25.9351 | 23.4774, 25.9245 | Gilf Kebir, Egypt (8 km) |
| 351 | S2A_49KGP_20240107_0_L2A | 20 m window | -23.9399, 113.7620 | -23.9440, 113.7521 | Lake MacLeod, Carnarvon (13 km) |
| 352 | S2A_11SMA_20240616_0_L2A | 80 m | 36.6302, -117.8019 | 36.6664, -117.8194 | Owens Lake (USA) (28 km) |
| 353 | S2A_44SKA_20240930_0_L2A | 80 m | 32.0569, 78.1990 | 32.0895, 78.2047 | Spiti Valley, India (24 km) |
| 354 | S2A_41RQJ_20240922_0_L2A | 80 m | 25.8355, 66.0077 | 25.8306, 65.9653 |  |
| 355 | S2B_11TLF_20240316_0_L2A | 80 m | 40.8453, -118.3015 | 40.7252, -118.3752 |  |
| 356 | S2B_33JYJ_20240402_0_L2A | 20 m window | -28.2432, 17.1378 | -28.2477, 17.1324 | Richtersveld, South Africa (13 km) |
| 357 | S2B_19KEP_20240427_0_L2A | 80 m | -23.6007, -68.0538 | -23.6407, -67.9588 | Lagunas Miscanti and Minique, Chile (27 km) |
| 358 | S2B_11SNA_20241009_0_L2A | 20 m window | 36.1485, -116.7088 | 36.1540, -116.7076 | Badwater Basin, Death Valley (10 km) |
| 359 | S2A_19JFM_20240519_0_L2A | 20 m window | -25.6896, -67.6912 | -25.6616, -67.6887 | Salar de Antofalla, Argentina (9 km) |
| 360 | S2B_36MZD_20240805_0_L2A | 20 m window | -1.8219, 36.2601 | -1.8266, 36.2598 | Lake Magadi, Kenya (5 km) |
| 361 | S2B_12SUE_20240615_0_L2A | 80 m | 35.6765, -112.0773 | 35.6516, -112.1019 |  |
| 362 | S2B_12VVJ_20230829_0_L2A | 20 m window | 56.9522, -111.3760 | 56.9455, -111.3794 | Athabasca oil sands mines, Alberta Canada (12 km) |
| 363 | S2B_14SKG_20240613_0_L2A | 20 m window | 37.5582, -101.4257 | 37.5627, -101.4286 | Southwest Kansas centre pivots, USA (22 km) |
| 364 | S2A_32KRE_20240824_0_L2A | 80 m | -18.1769, 12.7347 | -18.1625, 12.7486 |  |
| 365 | S2A_36NZJ_20240126_0_L2A | 80 m | 3.4272, 36.5015 | 3.4176, 36.5572 |  |
| 366 | S2A_36MZC_20230128_0_L2A | 80 m | -2.6398, 35.7129 | -2.6818, 35.8120 | Ol Doinyo Lengai, Tanzania (20 km) |
| 367 | S2A_14SKG_20241006_0_L2A | 20 m window | 37.7537, -101.3594 | 37.7595, -101.3545 | Southwest Kansas centre pivots, USA (18 km) |
| 368 | S2B_11SQR_20241003_0_L2A | 20 m window | 31.7955, -114.7295 | 31.8147, -114.7675 | Colorado River delta, Mexico (6 km) |
| 369 | S2B_14SKG_20240613_0_L2A | 20 m window | 37.5550, -101.2780 | 37.5653, -101.2878 | Southwest Kansas centre pivots, USA (12 km) |
| 370 | S2A_19JFM_20240519_0_L2A | 20 m window | -25.5863, -67.5062 | -25.5882, -67.5187 | Salar de Antofalla, Argentina (10 km) |
| 371 | S2A_53HLE_20240110_0_L2A | 20 m window | -31.9321, 132.9611 | -31.9253, 132.9609 | Lake MacDonnell, South Australia (11 km) |
| 372 | S2A_36NZJ_20240126_0_L2A | 80 m | 3.5219, 35.7472 | 3.4638, 35.7826 |  |
| 373 | S2B_12SVH_20241003_0_L2A | 20 m window | 38.2203, -111.2653 | 38.2231, -111.2701 | Capitol Reef / Waterpocket Fold, Utah USA (17 km) |
| 374 | S2B_11SNA_20241009_0_L2A | 80 m | 36.8380, -116.5718 | 36.7809, -116.5890 |  |
| 375 | S2B_18LZH_20241020_0_L2A | 80 m | -15.5551, -72.0288 | -15.7760, -71.8360 | Colca Canyon, Arequipa Peru (8 km) |
| 376 | S2A_50SPG_20240425_0_L2A | 20 m window | 37.7696, 119.1557 | 37.8119, 119.2729 | Yellow River Delta, China (7 km) |
| 377 | S2A_36SXH_20241004_0_L2A | 80 m | 38.2542, 35.0264 | 38.2740, 35.0791 |  |
| 378 | S2B_37RCK_20240602_0_L2A | 20 m window | 26.6084, 37.7800 | 26.5999, 37.7811 | AlUla sandstone (Saudi Arabia) (12 km) |
| 379 | S2A_41RQJ_20240515_0_L2A | 20 m window | 25.5552, 65.5737 | 25.5450, 65.5290 | Hingol / Makran ranges, Pakistan (8 km) |
| 380 | S2B_33RYH_20241004_0_L2A | 20 m window | 24.8850, 17.7073 | 24.8209, 17.6760 | Waw an Namus (Libya) (9 km) |
| 381 | S2B_44TLP_20241008_0_L2A | 20 m window | 44.0993, 78.9879 | 44.0884, 78.9823 | Aktau Mountains, Altyn-Emel Kazakhstan (12 km) |
| 382 | S2A_45TYH_20240623_0_L2A | 80 m | 43.0714, 90.3846 | 42.9501, 90.0631 |  |
| 383 | S2A_30SWG_20240621_0_L2A | 80 m | 37.4484, -2.0630 | 37.0920, -2.2248 |  |
| 384 | S2A_19JFM_20240110_0_L2A | 20 m window | -25.6858, -67.6557 | -25.6986, -67.6715 | Salar de Antofalla, Argentina (8 km) |
| 385 | S2B_13SCS_20240527_0_L2A | 80 m | 32.9718, -106.6170 | 32.8618, -106.9538 |  |
| 386 | S2A_45TTF_20240210_0_L2A | 20 m window | 41.0938, 84.5250 | 41.0872, 84.5743 | Tarim River poplar forest, China (11 km) |
| 387 | S2A_19JFG_20240419_0_L2A | 20 m window | -29.9974, -67.8346 | -29.9823, -67.8280 | Ischigualasto / Talampaya, Argentina (2 km) |
| 388 | S2A_40QEL_20240524_0_L2A | 80 m | 23.3669, 57.6569 | 23.3460, 57.7319 | Hajar Mountains ophiolite, Oman (15 km) |
| 389 | S2B_38KNA_20241108_0_L2A | 80 m | -22.4559, 45.6942 | -22.4750, 45.6999 |  |
| 390 | S2B_11SNA_20240611_0_L2A | 80 m | 36.3154, -116.1518 | 36.3284, -116.1417 |  |
| 391 | S2B_18LUK_20241225_0_L2A | 80 m | -13.6132, -75.8455 | -13.6317, -75.8769 |  |
| 392 | S2B_19KFP_20240407_0_L2A | 80 m | -24.2664, -67.5601 | -24.2553, -67.5690 |  |
| 393 | S2B_49KGP_20240809_0_L2A | 20 m window | -23.9464, 113.7212 | -23.9611, 113.7208 | Lake MacLeod, Carnarvon (9 km) |
| 394 | S2B_11SNA_20240611_0_L2A | 20 m window | 36.2092, -116.9665 | 36.2082, -116.9618 | Death Valley (Badwater Basin), California USA (11 km) |
| 395 | S2A_11TMN_20240612_0_L2A | 80 m | 47.6187, -118.0226 | 47.6060, -118.0042 |  |
| 396 | S2B_11SPS_20240229_0_L2A | 80 m | 33.3425, -115.2362 | 33.2706, -115.2596 |  |
| 397 | S2A_13SCS_20240909_0_L2A | 80 m | 33.4056, -106.4524 | 33.4185, -106.4291 |  |
| 398 | S2A_47SNA_20240926_0_L2A | 80 m | 36.7646, 99.3773 | 36.7887, 99.3807 |  |
| 399 | S2B_11SLC_20240604_0_L2A | 80 m | 38.2435, -118.8542 | 38.0882, -118.9060 | Mono Lake, California (21 km) |
| 400 | S2B_36MXC_20240828_0_L2A | 80 m | -2.6433, 34.0051 | -2.6806, 34.0388 |  |
| 401 | S2B_13SDT_20240527_0_L2A | 80 m | 34.2375, -105.5244 | 34.1931, -105.5054 |  |
| 402 | S2B_48TUP_20240521_0_L2A | 80 m | 43.4136, 102.5579 | 43.4998, 102.7193 |  |
| 403 | S2B_32RMP_20240608_0_L2A | 80 m | 25.7998, 8.2842 | 25.6692, 8.2176 | Tassili n'Ajjer (Algeria) (16 km) |
