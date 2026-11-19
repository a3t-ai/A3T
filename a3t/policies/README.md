# A3T policies (config-to-code)

What A3T ships for authorization, reviewed like code and compiled into each
FlowR release:

| File | What it is |
| --- | --- |
| `catalogue.yaml` | Every permission a role or rule can name, and what enforces it |
| `baseline.cedar` | The A3T baseline: deny by default; applies everywhere, beneath every org policy |
| `a3t.cedarschema`, `a3t.schema.json` | The Cedar schema every policy is validated against (keep the two in step) |

Orgs never edit these. An org's own roles and rules are YAML files in the
org's policy repo (`kind: a3t/role`, `kind: a3t/policy`), compiled to Cedar
`forbid` statements by `a3t-security/src/policy_files.rs`: they can only narrow
what the baseline allows.
