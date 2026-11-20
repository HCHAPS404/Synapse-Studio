"""ROS 2 wrapper of the `@MODULE@` module, written by `neuron emit python`.

`python @MODULE@.py --ros2` runs this file. One node, named `@MODULE@`:

* every `signal` is published on `neuron/@MODULE@/<signal>` whenever it changes;
* every input the program reads is subscribed on `neuron/@MODULE@/in/<input>` (dots in
  the name become underscores) and driven into the program when a message arrives;
* every event a task waits for and the program never raises itself is subscribed on
  `neuron/@MODULE@/event/<event>` (`Bool`): a `true` message raises the event;
* a timer fires every 1 ms of wall time and advances the program by one base tick.

The message types come from `std_msgs` (`Bool`, `Int64`, `Float64`, `String`). They are
imported when the node starts, so the other modes of the program never need ROS 2.
Quantities are published in base units (m, s, kg ...), like the program computes them.

This wrapper is verified against a stub `rclpy` only. Confirm it on a real ROS 2
installation before relying on it.
"""

import importlib
import sys

import neuron_rt as rt
import @MODULE@ as prog

NODE_NAME = "@MODULE@"
SIGNALS = [@SIGNALS@]
INPUTS = [@INPUTS@]
EVENTS = [@EVENTS@]
TICK_S = rt.Runtime.BASE_TICK_NS / 1.0e9
_TYPES = {"bool": "Bool", "int": "Int64", "float": "Float64", "str": "String"}


def _msg_type(kind):
    """The `std_msgs` class that carries values of `kind` (imported on first use)."""
    return getattr(importlib.import_module("std_msgs.msg"), _TYPES[kind])


def _to_msg(kind, value):
    if kind == "bool":
        return bool(value)
    if kind == "int":
        return int(value)
    if kind == "float":
        return float(value)
    return rt.show(value)


def _topic(name):
    return "neuron/%s/%s" % (NODE_NAME, name)


def _in_topic(name):
    return "neuron/%s/in/%s" % (NODE_NAME, name.replace(".", "_"))


def _event_topic(name):
    return "neuron/%s/event/%s" % (NODE_NAME, name.replace(".", "_"))


def main(argv=None):
    try:
        import rclpy
        from rclpy.node import Node
    except ImportError:
        sys.stderr.write(
            "rclpy was not found: install ROS 2 and source its setup script, then run again\n"
        )
        return 2
    prog._boot()
    run = prog.R
    kinds = dict(SIGNALS)

    class NeuronNode(Node):
        def __init__(self):
            super().__init__(NODE_NAME)
            self.pubs = {}
            self.subs = []
            for name, kind in SIGNALS:
                self.pubs[name] = self.create_publisher(_msg_type(kind), _topic(name), 10)
            for name, kind in INPUTS:
                self.subs.append(
                    self.create_subscription(
                        _msg_type(kind), _in_topic(name), self._input(name, kind), 10
                    )
                )
            for name in EVENTS:
                self.subs.append(
                    self.create_subscription(
                        _msg_type("bool"), _event_topic(name), self._event(name), 10
                    )
                )
            run.listener = self._publish
            self.timer = self.create_timer(TICK_S, self._tick)

        def _input(self, name, kind):
            def on_message(msg):
                run.drive(name, _to_msg(kind, msg.data))

            return on_message

        def _event(self, name):
            def on_message(msg):
                if msg.data:
                    run.external_event(name)

            return on_message

        def _publish(self, name, value):
            kind = kinds.get(name)
            if kind is None:
                return
            try:
                data = _to_msg(kind, value)
            except (TypeError, ValueError):
                return  # a value the message type cannot carry, such as an unset input
            msg = _msg_type(kind)()
            msg.data = data
            self.pubs[name].publish(msg)

        def _tick(self):
            run.run_until(run.now + rt.Runtime.BASE_TICK_NS)

    rclpy.init(args=argv)
    node = NeuronNode()
    try:
        rclpy.spin(node)
    except KeyboardInterrupt:
        pass
    node.destroy_node()
    rclpy.shutdown()
    return 0
