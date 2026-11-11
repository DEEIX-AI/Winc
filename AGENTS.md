# Winc development

- This is a Rust workspace using the toolchain pinned in `rust-toolchain.toml`.
- Read `.github/CONTRIBUTING.md` before changing code. Branch from `dev` and target PRs at `dev`.
- Keep changes focused; preserve existing work and avoid new dependencies unless needed.
- Application state and input live in `crates/winc-app/src/app`, drawing in
  `crates/winc-app/src/view`, image decoding in `crates/winc-preview`, and rasterization in
  `crates/winc-render`. Interface text belongs in both `winc-i18n` language bundles.
- Run locally with `cargo run --release -- <path>`.
- Add behavior tests before implementation. UI regression tests use the offscreen renderer in
  `crates/winc-app/src/app/snapshot`; these do not replace native window interaction checks.
- Required checks: `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo xtask test --all`, and
  `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`.
- Use `cargo xtask test` for changed crates during development. Run `cargo xtask bench` when
  startup, memory, or scrolling may be affected; report measured results and environment limits.
- Test image interactions in full preview and SVG split view, including overlay interception,
  pointer anchoring, clipping, cache bounds, and switching documents. Preserve source editing.
- Commit subjects and PR titles use ASCII `type: subject`, without a parenthesized scope.
- Keep validation evidence in the PR; distinguish offscreen tests, native macOS checks, and CI
  on other platforms. Do not sign contributor agreements on the user's behalf.
