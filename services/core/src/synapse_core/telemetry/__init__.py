"""Telemetry ingest, query and retention (E4.4, E4.5).

Plain Postgres tables on Neon, no TimescaleDB (design decision 3). `ingest.py` turns relay
`telemetry` frames into `telemetry_samples` rows with a storage cap; `query.py` serves
`GET /v1/telemetry`; `retention.py` rolls samples into `telemetry_1m` and deletes expired
rows under a Postgres advisory lock.
"""
