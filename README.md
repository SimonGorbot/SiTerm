## Command List

All commands follow the general format: `protocol action payload`. Payloads are action specific depending on the command you are writting.

### I2C

#### Leader

##### Single Byte Read

 Protocol  | Action  | Payload                            | Example                        | Complete |
---------- |-------- |------------------------------------|--------------------------------|----------|
i2c        |r        |device_address register_address 1   |`i2c r 0x1A 0x0F 1`             | ✅       |

##### Single Byte Write

 Protocol  | Action  | Payload                                         | Example                         | Complete |
---------- |-------- |-------------------------------------------------|---------------------------------|----------|
i2c        |w        |device_address register_address value_to_write   |`i2c r 0x1A 0x0F 0xFF`           | ✅       |

##### Batch Read


 Protocol | Action  | Payload                                          | Example              | Complete |
----------|---------|--------------------------------------------------|----------------------|----------|
i2c       |r        |device_address starting_register_address num_reads|`i2c r 0x1A 0x0F 3`   |🚧        |

_Note: Only works on devices with auto-increment. Future versions might have config options for how to enable incrementing of registers._

##### Batch Write

 Protocol | Action  | Payload                                                                                | Example                           | Complete |
----------|---------|----------------------------------------------------------------------------------------|-----------------------------------|----------|
i2c       |r        |device_address starting_register_address value_to_write_1 ... value_to_write_n          |`i2c r 0x1A 0x0F 0x0A 0x0B 0x0C`   |🚧        |

_Note: Only works on devices with auto-increment. Future versions might have config options for how to enable incrementing of registers._

#### Follower

##### Listen

*coming soon*

### SPI

#### Leader

##### Single Byte Read

*coming soon*

##### Single Byte Write

*coming soon*

##### Batch Read

*coming soon*

##### Batch Write

*coming soon*

### UART

#### Send String

*coming soon*

#### Send Bytes

*coming soon*

#### Read Number Bytes

*coming soon*

#### Read Until Byte/Bytes

*coming soon*

### PWM

#### Set Duty Cycle

*coming soon*
