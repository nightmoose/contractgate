# ContractGate Validate (GitHub Action)

Compile every ContractGate YAML under `contracts/` in CI. Local, no network,
no CLI release tarball. Uses `pip install contractgate`.

```yaml
- uses: actions/checkout@v4
- uses: nightmoose/contractgate/actions/validate@main
  with:
    path: contracts
```

| Input | Default | Meaning |
|---|---|---|
| `path` | `contracts` | Directory to glob for `*.yaml` / `*.yml` |
| `python-version` | `3.12` | Python used to run the SDK |
| `allow-empty` | `false` | Succeed if the directory has no YAML |

Pin `@main` to a commit SHA once you depend on this in production.

Full example workflow: [`docs/examples/github-actions/contractgate.yml`](../../docs/examples/github-actions/contractgate.yml).
