# Security Policy

## Credentials & secrets

openintel reads all credentials **only from environment variables** (see
[`.env.example`](.env.example)). It never reads them from a committed file and
never writes them to disk. Secrets are wrapped in `secrecy::SecretString`
(redacted in debug output, zeroized on drop) and are never logged. When
openintel runs as an MCP server, the credentials stay in its process
environment — the connected AI agent never sees them.

**Never commit real credentials:**

- Use a gitignored `.env` (already in `.gitignore`) or export the variables in
  your shell. Do **not** `git add -f` a `.env`.
- Never hardcode a secret in source — the env-only design exists so you never
  have to, and code review should reject any hardcoded key.
- Prefer a gitignored `.env` + `direnv` over exporting inline (which lands in
  shell history).

CI runs a [`gitleaks`](https://github.com/gitleaks/gitleaks) secret scan over
the full git history on every push and pull request; an accidentally-committed
key fails the build before it can merge.

## Reporting a vulnerability

Please report security issues privately via GitHub's
[private vulnerability reporting](https://github.com/Kloudy-Sky/openintel/security/advisories/new)
rather than opening a public issue. We will acknowledge and respond as soon as
we can.

## Scope reminder

openintel is **analysis-only**: it never executes trades, touches a broker, or
holds broker credentials. Trade execution happens only through your broker's own
MCP, gated by that broker's controls and your approval. Read the README's risk
section before connecting one.
