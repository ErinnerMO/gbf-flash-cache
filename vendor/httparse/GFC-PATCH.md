# Local httparse patch

Source: httparse 1.10.1 from crates.io, https://github.com/seanmonstar/httparse.
The original source, build script, README and licenses are retained. The manifest
omits upstream integration tests, benchmarks and their development dependencies.

`src/lib.rs::parse_uri` rejects literal `#` in request targets before Hyper passes
them to `http::Uri`, which otherwise silently truncates the fragment. Encoded
`%23`, header values and message bodies are unaffected. The check applies after
all SIMD parsing paths and to every request on persistent connections.

This local change is covered by the raw gateway regression in
`core/tests/network.rs`. Remove the patch when the upstream dependency stack
rejects these request targets before URI conversion.
