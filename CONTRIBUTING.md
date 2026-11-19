# Contributing to Catwright

Thanks for taking the time. Bug reports, ideas, code and translations are
all welcome.

Catwright is maintained by one person in their spare time, so answers can
take a few days. Please be patient, and follow the
[code of conduct](CODE_OF_CONDUCT.md).

## Reporting bugs and asking for features

Open an [issue](https://github.com/FreshKnoppers/Catwright/issues/new/choose)
and pick the matching form. For bugs, the Catwright version
(`catwright --version`), the Vibe version (`vibe --version`) and your
distribution help most.

Problems in Mistral Vibe or `vibe-acp` itself belong to the
[Mistral Vibe project](https://github.com/mistralai/mistral-vibe).

Security problems are not reported in public issues; see
[SECURITY.md](SECURITY.md).

Do not paste API keys, private prompts or file contents you do not want to
publish into an issue.

## Before you write code

For anything bigger than a small fix, open an issue first and describe what
you want to change. That saves you work on something that does not fit.

## Building and testing

You need a stable Rust toolchain. All cargo commands run in `app/`:

```sh
cd app
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

These are the same checks CI runs on every push and pull request; all three
must pass. The tests run headless against recorded ACP traffic and need
neither Vibe nor a display.

Useful reading:

- `app/MECHANICS.md`: how the app works inside
- `docs/acp.md`: the part of the Agent Client Protocol the app implements
  (the source of truth for the protocol)
- `docs/design.md`: colours and pixel style
- `AGENTS.md`: the project rules in short form (written for coding agents,
  but they apply to everyone)

## Rules

- **No new dependencies** in `Cargo.toml` without discussing it in an issue
  first. Builds run offline.
- **Privacy first.** Never log or print API keys, prompts or file contents.
  No telemetry and no network calls the user did not trigger. Catwright
  never handles Vibe API keys.
- **No placeholders.** No stubs, fakes or "TODO: implement" logic in
  `app/src/`. If something cannot be done properly, say so in the issue or
  pull request.
- **Tests stay.** Do not weaken, delete or `#[ignore]` existing tests. New
  behaviour gets a test where that is reasonable.
- `app/tests/fixtures/acp/` holds real ACP wire transcripts; do not edit
  them.
- Code, comments and identifiers are English.

## UI text and translations

UI text is written in English in the code and shown through
`crate::i18n::tr("…")`, or `trf("… {name} …", &[("name", &value)])` for
text with values. Never build UI text from bare string literals.

Every such text needs a German entry in `app/i18n/de.po`; a test fails
otherwise. If you do not speak German, say so in the pull request and the
maintainer adds the translation.

New languages are welcome: add a catalog next to `app/i18n/de.po` and
register the language in `app/src/i18n.rs` (`Lang`).

## UI changes

Look at what you changed before you open the pull request, and attach a
screenshot. Rendering needs no display or GPU:

```sh
catwright --screenshot out.png --headless --project ~/some/project
```

`tools/screenshots/render.sh` renders the README screenshots against demo
sessions. Do not commit screenshots rendered by tests.

## Commits and pull requests

- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/):
  `feat(scope): …`, `fix(scope): …`, `docs: …`, `chore: …`. Describe what
  changes for the user.
- Keep a pull request to one topic.
- Changes users notice get a line in the `Unreleased` section of
  `CHANGELOG.md`.

## License

Catwright is licensed under either the [MIT License](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), at your option. Unless you explicitly
state otherwise, any contribution you submit is licensed under the same two
licenses.
