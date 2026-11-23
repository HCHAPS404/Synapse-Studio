"""A stand-in for `avrdude`, so the Flasher tests need no real tool or board.

Exits 0 and echoes its arguments. If the environment variable `FAKE_AVRDUDE_FAIL` is set it
exits 1, so a test can exercise the failure path. If `FAKE_AVRDUDE_HANG` is set it sleeps
long enough to trip the timeout.
"""

import os
import sys
import time

if os.environ.get("FAKE_AVRDUDE_HANG"):
    time.sleep(30)
print("fake avrdude: " + " ".join(sys.argv[1:]))
sys.exit(1 if os.environ.get("FAKE_AVRDUDE_FAIL") else 0)
