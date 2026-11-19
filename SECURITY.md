# Security policy

## Reporting a vulnerability

Please do not open a public issue for security problems.

Report them privately through GitHub instead: open the repository's
**Security** tab and choose **Report a vulnerability**. Only the
maintainer can see the report. We can discuss and fix it there and
publish an advisory once a fixed release is out.

Helpful details:

- the Catwright version (`catwright --version` or the release name) and
  the Vibe version
- what an attacker controls (for example agent output, a file in the
  project, a crafted session) and what they gain
- steps or a minimal example that reproduce it

Catwright is maintained by one person in their spare time. You will get an
answer, but there is no fixed response time.

## Supported versions

Only the latest release gets security fixes.

## Scope

In scope is Catwright itself, for example:

- how it handles messages from `vibe-acp` (paths, tool calls, approvals)
- rendering of agent output (Markdown, links, diffs, previews)
- files it reads or writes, and the permissions of stored settings
- the release archive and `dist/install.sh`

Problems in Mistral Vibe or `vibe-acp` belong to the
[Mistral Vibe project](https://github.com/mistralai/mistral-vibe). If you
are unsure where a problem belongs, report it here and we will sort it out.

## Verifying a download

Each release archive comes with a `.sha256` file:

```sh
sha256sum -c catwright-vX.Y.Z-x86_64-linux.tar.gz.sha256
```

From v0.4.0 on, releases also carry a build provenance attestation, which
shows that the archive was built by this repository's release workflow:

```sh
gh attestation verify catwright-vX.Y.Z-x86_64-linux.tar.gz --repo FreshKnoppers/Catwright
```
