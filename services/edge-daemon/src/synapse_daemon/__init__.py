"""SYNAPSE Studio Edge Daemon.

Talks to field devices behind one `Driver` interface, relays telemetry and trace to the
Core API over the Neuron Wire relay protocol, and serves the PWA on the LAN. Every driver
is tested against a simulated peer; the daemon runs with no cloud and no real hardware.

Safety: the software E-STOP is supplementary. A hardwired E-STOP is the authority. Nothing
in this package claims IEC 61508 or ISO 13849 compliance.
"""

__version__ = "0.1.0"
