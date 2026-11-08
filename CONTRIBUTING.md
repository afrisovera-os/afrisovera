# Contributing to AfriSovera OS

> **TODO (placeholder):** the full contributing guide is being drafted in the project's policy work and
> will replace this file. Until then, the short rules below apply.

1. **Talk first for big changes.** Open an issue describing the problem and the design before writing a
   large patch.
2. **Tests come with the change.** A kernel or driver change adds or extends a suite in
   `scripts/<name>-test.sh` that fails without the change and passes with it, and registers it in
   `scripts/run-all-tests.sh`. The existing suites must stay green (CI runs them).
3. **Keep commits small and explained.** Say what changed and why; reference the issue.
4. **Licence.** By contributing you agree that your contribution is licensed under the Apache License 2.0
   (see `LICENSE`), with no additional terms. Sign off each commit (`git commit -s`) to certify the
   [Developer Certificate of Origin](https://developercertificate.org/).
5. **Be kind.** The [Code of Conduct](CODE_OF_CONDUCT.md) applies everywhere in the project.

Security problems: see [SECURITY.md](SECURITY.md). Never open a public issue for one.
