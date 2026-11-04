<!-- Open pull requests against `dev`. Use a `type: subject` title, for example
     `fix: keep the cursor column after undo`. -->

## Summary

<!-- What does this change, and why? Link the issue, for example "Fixes #123". -->

## Change type

- [ ] Bug fix
- [ ] Feature
- [ ] Performance
- [ ] Refactor
- [ ] Documentation
- [ ] Build / tooling

## Verification

<!-- Check what you ran, and name the platforms you tested on. -->

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo xtask test --all`
- [ ] Tested by hand on: <!-- macOS / Linux / Windows -->

## Screenshots

<!-- For visible changes, add before and after screenshots. Delete this section otherwise. -->

## Checklist

- [ ] The change is focused and does not include unrelated refactoring or formatting.
- [ ] Tests cover the fixed bug or the new behavior.
- [ ] New interface text is added in both English and Simplified Chinese.
- [ ] Startup time, memory use, and large files are not made slower (run `cargo xtask bench` when in doubt).
- [ ] I have read and agree to the [Contributor License Agreement](https://github.com/DEEIX-AI/Winc/blob/HEAD/.github/CLA.md).
