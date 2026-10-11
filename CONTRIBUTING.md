# Contributing to Heronite

Thanks for helping! Bug reports, ideas, and pull requests are all welcome.

- **Questions and ideas:** start a thread in [Discussions](https://github.com/RafaelGoulartB/Heronite/discussions).
- **Bugs and feature requests:** open an [issue](https://github.com/RafaelGoulartB/Heronite/issues/new/choose) with one of the templates.
- **Security problems:** report them privately, as described in [SECURITY.md](SECURITY.md).

## Build

Heronite is Linux only. You need a stable Rust toolchain, `clang` and `lld` (the linker set in `.cargo/config.toml`), and the development packages for xkbcommon, xcb, Wayland, udev, fontconfig, freetype, GTK 3, and WebKitGTK 4.1. On Ubuntu, [`scripts/ci-deps.sh`](scripts/ci-deps.sh) installs everything CI uses.

```bash
# fast optimised build for trying the app
cargo run --profile quick -p heronite

# what CI runs; all three must pass
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Point `HERONITE_DATA_DIR` at a copy of a data folder to try changes without touching your real library. `HERONITE_DEBUG_*` variables open a page, dialog, or menu at startup, which helps with screenshots.

## Code

- Read [ARCHITECTURE.md](docs/ARCHITECTURE.md) for the crates and patterns, [CODE-STYLE.md](docs/CODE-STYLE.md) for the code rules, and [UI-CONVENTIONS.md](docs/UI-CONVENTIONS.md) for the look of the UI.
- Format with `cargo fmt --all` (settings in `rustfmt.toml`). Clippy must report zero warnings.
- Library crates in `crates/` never depend on GPUI; slow work runs off the UI thread.
- UI text is English.

## Commits and pull requests

- Write commit messages in the [Conventional Commits](https://www.conventionalcommits.org/) style used in the history, such as `feat(stats): …`, `fix(epic): …`, or `docs(readme): …`.
  The release notes are built from them ([`cliff.toml`](cliff.toml)): `feat`, `fix`, `perf`, `ui`, and `docs` commits are listed; `refactor`, `style`, `test`, `ci`, `build`, and `chore` are not. Write the message for the person reading the release notes.
- Keep a pull request to one change, and say how you tested it. For UI changes, add a screenshot or a short video.
- New third-party files (icons, fonts, images) need an entry in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) and a GPL-compatible license.

## Releases

Merging a pull request with one of these labels publishes a new version:

| Label | When | Example |
|---|---|---|
| `release:patch` | bug fixes only | 0.1.0 → 0.1.1 |
| `release:minor` | new features or changed behaviour | 0.1.0 → 0.2.0 |
| `release:major` | big or breaking changes | 0.1.0 → 1.0.0 |

The release workflow bumps the version in `Cargo.toml`, tags it, builds the AppImage, and publishes a GitHub release. A pull request without a release label is merged without a new version. Maintainers can also start a release from **Actions › Release › Run workflow**.

By contributing, you agree that your work is licensed under the [GPL-3.0](LICENSE).
