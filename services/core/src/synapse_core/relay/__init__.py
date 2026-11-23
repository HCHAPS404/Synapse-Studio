"""Relay and twin gateway (E4): a WebSocket endpoint inside the Core API.

Implements `spec/protocols/relay.md` v1. The JSON Schemas in `spec/schemas/relay/` are the
single source of truth; every inbound frame is validated against them before the server
acts on it (`validate.py`). The role table of the markdown is `roles.py`; a test parses the
markdown and compares it with `ROLE_MESSAGES` so the two cannot drift. The hub, subscription
cache and fan-out are `hub.py`; the per-connection lifecycle is `session.py`.
"""
