# brogz

Measure wire bytes and timing for `identity`, `gzip`, and `brotli` responses across a
site's assets. One static binary, no Node, no `curl` in your `PATH`.

> `br` + `gz` — the two encodings we compare.

## Install (eventually)

```bash
cargo install brogz
# or grab a prebuilt binary from the GitHub Releases page
```

## Usage (planned)

```bash
brogz https://app.example                # auto-discover assets from /index.html
brogz https://app.example --runs 20      # 20 probes per URL × encoding (default 10)
brogz https://app.example -o report.json # write a JSON report alongside the table
brogz https://app.example -p /index.html -p /assets/app.js   # skip discovery
brogz https://app.example -k             # --insecure, skip TLS verification
```

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual-licensed as above, without any additional terms or conditions.
