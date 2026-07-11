# FIXME

- Security/config hygiene: local `config.json` and an existing review note contain real API-key material. I did not rewrite or delete `config.json` because this project's AGENTS.md explicitly marks it as required local state. Rotate the exposed key, then replace local secret values with placeholders or move them to an ignored local override.
