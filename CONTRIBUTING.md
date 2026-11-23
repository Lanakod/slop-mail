# Contributing to Slop Mail

Issues and pull requests are welcome in English or Russian. Search existing issues first. For a bug, include your app version, operating system, account protocol and the smallest set of steps that reproduces it. For a feature, describe the mail workflow you want to improve. [Open an issue](https://github.com/Lanakod/slop-mail/issues/new/choose)

Use a disposable profile and test messages when collecting logs or screenshots. Remove credentials, email addresses and message contents before attaching them to an issue.

## Testing a change

Follow the platform setup in the [workshop](README.md#workshop), then run the same checks used by CI.

```sh
npm ci
npm run lint
npm run typecheck
npm test
npm run build:ui
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Mail integration tests use temporary profiles and local IMAP, POP3 and SMTP servers. Describe what you tested and on which platform in the pull request. Add an entry to the next release section in [CHANGELOG.md](CHANGELOG.md) for a user-visible change. The client displays these entries after updates.
