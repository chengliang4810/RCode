# stacksafe-macro

This is an implementation crate for the [`stacksafe`](https://crates.io/crates/stacksafe) library.

**Please refer to the main [`stacksafe`](https://crates.io/crates/stacksafe) crate for documentation and usage examples.**

This crate contains the procedural macro that powers the `#[stacksafe]` attribute and should not be used directly.

## RCode local maintenance

This directory vendors the crates.io release `stacksafe-macro` 0.1.4 from
<https://crates.io/crates/stacksafe-macro/0.1.4>, whose repository is
<https://github.com/fast/stacksafe> (commit
`a5c029b7d3a5fe6771ecd79266b6e21313cfa174`, path `stacksafe-macro`). The source
was copied from the local Cargo registry; no upstream repository checkout is
required to build this patch.

The local change replaces `proc-macro-error2` with `proc-macro-error3` 3.1.1,
disables its default features, enables `syn2-error`, and updates the three error
helper imports. The stack-growth macro generation remains unchanged.

The published manifest licenses this crate under Apache-2.0. Its release archive
omits a license file, so `LICENSE-APACHE` supplies the standard Apache License 2.0
text from the locally cached `anstyle-wincon` 3.0.11 package. No upstream copyright
notice has been removed or replaced.
