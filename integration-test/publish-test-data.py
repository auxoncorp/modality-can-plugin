#!/usr/bin/env python3
# python3 -m pip install cantools

import os
import cantools
import can
from pprint import pprint

iface = os.getenv('TEST_CAN_INTERFACE', 'vcan0')
pprint(iface)

db = cantools.database.load_file(os.getenv('TEST_CAN_DBC', 'test.dbc'))

msg0 = db.get_message_by_name('test_msg0')
pprint(msg0)
pprint(msg0.signals)

msg1 = db.get_message_by_name('test_msg1')
pprint(msg1)
pprint(msg1.signals)

can_bus = can.interface.Bus(iface, bustype='socketcan')

# Send msg0 with muxed_a
data = msg0.encode({
    'u4_le': 10,
    'u4_be': 11,
    'i4_le': -2,
    'i4_be': -3,
    'float32_le': -123.4,
    'i8_le_so': -4,
    'muxer0': 0,
    'muxed_a': -2,
    'flag0': 1,
    'enum0': 'value2',
})
pprint(data)
message = can.Message(arbitration_id=msg0.frame_id, data=data)
can_bus.send(message)

# Send msg0 with muxed_b
data = msg0.encode({
    'u4_le': 11,
    'u4_be': 12,
    'i4_le': -3,
    'i4_be': -4,
    'float32_le': 123.4,
    'i8_le_so': 4,
    'muxer0': 1,
    'muxed_b': -4,
    'flag0': 0,
    'enum0': 'value0',
})
pprint(data)
message = can.Message(arbitration_id=msg0.frame_id, data=data)
can_bus.send(message)

# Send msg1
data = msg1.encode({
    'double_be': 2345.6
})
pprint(data)
message = can.Message(arbitration_id=msg1.frame_id, data=data)
can_bus.send(message)

# Send a message not contained in the dbc file
message = can.Message(arbitration_id=0x006, data=[])
can_bus.send(message)

can_bus.shutdown()
