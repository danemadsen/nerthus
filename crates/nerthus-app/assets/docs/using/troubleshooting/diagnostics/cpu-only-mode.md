# CPU Only Mode

In some cases, Nerthus may have trouble using your GPU device. Often this is because it is an unsupported older device or may have outdated drivers.

To enable CPU only mode, open Diagnostics (Help › Diagnostics…, or `nerthus --diagnostics` from a terminal if Nerthus does not start properly). Select "Run Nerthus with CPU only" and click "Start".

Nerthus starts again in CPU only mode. It will now remain in CPU only mode, including when running the command-line builds (`nerthus-build`).

To use CPU only mode for a single session instead, start Nerthus with `nerthus --cpu-only`, or a build with `nerthus-build build --cpu-only`.

To re-enable multiple device usage, you can go to the [Compute](../../../ui/interface/options/compute.md) tab in [Options](../../../ui/interface/options/index.md).
