# Security policy

RedOx is pre-alpha. Report vulnerabilities privately via GitHub's "Report a vulnerability"
(Security tab) rather than a public issue. Include a minimal reproduction and the affected
crate. Expect a first response within 7 days.

Areas most likely to matter: `unsafe` code in `redox_ecs` and `redox_asset`, and parsers for
untrusted files (glTF, images, world files).
