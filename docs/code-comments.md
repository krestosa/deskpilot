<!-- File purpose: Defines the repository commenting standard and the narrow exclusions required for generated or legally exact files. -->
# Code commenting standard

Every project-maintained text file contains a concise file-purpose comment. Every Rust or PowerShell function contains a nearby function-purpose comment that explains its responsibility rather than restating its syntax.

`Cargo.lock` is excluded because Cargo generates it and manual comments would invalidate the lockfile format. `LICENSE.md` is excluded because CI verifies it as the byte-identical official PolyForm Strict License 1.0.0 text. Binary outputs and generated `target`, `dist`, log, crash-report, and artifact directories are not source files and are not committed.
