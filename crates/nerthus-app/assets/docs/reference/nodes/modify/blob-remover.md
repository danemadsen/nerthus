# BlobRemover

A low-level node that helps isolate and remove small blobs or disconnected shapes.

## When to Use It
Use BlobRemover when masks or heightfields contain small stray islands that should be cleaned up without changing the main shapes.

## Properties

#### Range

Controls the sensitivity of the blob detection. Higher values detect a wider range of blob sizes or intensities.

#### Connectivity

Specifies how connected blobs must be to be considered a single shape. Higher connectivity allows blobs to be more loosely attached to larger shapes.

#### Enforce quantization

If enabled, forces the use of quantization during blob detection, improving efficiency and consistency in identifying blobs.

#### Quantization

Sets the quantization level for blob detection, with values between 1 and 128. Lower values provide finer detail, while higher values group similar values together for faster and more efficient blob identification.
