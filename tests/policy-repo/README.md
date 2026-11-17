# Test policy repo

A stand-in for an org's own GitHub policy repo, used to test FlowR's org
policy sync (`/security/orgs/{org}/source|sync|files`) on vc-dev. Each folder
is one org's files, the layout FlowR reads:

- `policies/*.yaml`: `kind: a3t/policy`, rules compiled to Cedar `forbid`s
- `roles/*.yaml`: `kind: a3t/role`, a baseline role minus permissions
- `access/<project>.yaml`: `kind: a3t/access`, IdP group or account to role

`acme/` is for org `acme` (projects `acme-p1`, `acme-sam1`). Connect it with
source `{repo: srirammails/a3t, ref: <branch>, folder: tests/policy-repo/acme}`.
