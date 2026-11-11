# Development regression notes

## 2026-11-11: Image zoom and pan

- A full workspace run caught `app_logic_does_not_reference_the_view`: the application-local
  module name `view` matched the repository's dependency-layer check. Renamed it `viewport`,
  preserving the existing check. Run `cargo xtask test --all`, including xtask's architecture tests.
- Review found stale picture drags after image replacement or leaving and returning to a tab.
  Resetting zoom alone left the old drag origin active. Synchronizing a new picture and leaving
  preview now cancel picture dragging; regression tests exercise both transitions.
- Opening quick-open during a drag previously allowed the image underneath to keep moving.
  Picture dragging now cancels when an overlay takes input; the regression opens the real picker
  after mouse-down and verifies no pan occurs.
