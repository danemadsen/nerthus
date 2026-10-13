# Halftone

Converts the terrain or mask into a halftone-style rendered output.

## When to Use It
Use Halftone when you want a terrain or mask drawn as a halftone pattern, for stylized output or overlays.

## Properties

#### Pattern

Specifies the halftone pattern or dithering algorithm used to convert a black and white map into halftone shading. Each pattern produces a distinct visual effect, simulating various print or screen halftone styles.

- **FloydSteinberg**: Applies Floyd-Steinberg error diffusion dithering, producing a classic, high-quality halftone effect with smooth gradients.
- **Riemersma**: Uses Riemersma dithering, which distributes error along a space-filling curve for a unique, less directional halftone appearance.
- **O2X2**: Uses a 2x2 ordered dithering matrix to create a small, repeating halftone pattern.
- **O3X3**: Uses a 3x3 ordered dithering matrix for a slightly larger, repeating halftone pattern.
- **O4X4**: Uses a 4x4 ordered dithering matrix, producing a more detailed and less repetitive halftone effect.
- **O8X8**: Applies an 8x8 ordered dithering matrix for fine, detailed halftone shading with minimal visible repetition.
- **H6X6A**: Uses a 6x6 halftone matrix (variant A) for a larger, more complex dot pattern.
- **H8X8A**: Applies an 8x8 halftone matrix (variant A), generating a dense and detailed halftone texture.
- **H6X6O**: Applies a 6x6 halftone matrix (variant O), producing a unique halftone pattern with distinct dot placement.
- **H8X8O**: Uses an 8x8 halftone matrix (variant O), providing a dense and alternative halftone style.
- **C5X5B**: Uses a 5x5 circular halftone matrix with black dots, simulating traditional print halftone screens.
- **C5X5W**: Uses a 5x5 circular halftone matrix with white dots, inverting the typical halftone dot style.
- **C6X6B**: Applies a 6x6 circular halftone matrix with black dots for a slightly larger dot pattern.
- **C6X6W**: Uses a 6x6 circular halftone matrix with white dots, providing an inverted halftone effect.
- **C7X7B**: Applies a 7x7 circular halftone matrix with black dots, creating a larger, more pronounced dot pattern.
- **C7X7W**: Uses a 7x7 circular halftone matrix with white dots, offering a bold, inverted halftone style.

#### Posterize

#### Depth
