// Provide a mapping for the controller GPIO pins

//
//| GPIO # | Name        | Description                                                    |
//| ------ | ----------- | -------------------------------------------------------------- |
//| 0      | RGBLED      | Data output for the five SK6805 (WS2812 style) RGB LEDs.       |
//| 1      | N/C         | Not connected.                                                 |
//| 2      | PWR_BTN_IN  | Input from the physical power button. Active low.              |
//| 3      | USER_BTN    | Input from the user-defined button. Active low.                |
//| 4      | PCIESLEEP   | Pull high to put the PCIe device to sleep.                     |
//| 5      | GPIO05      | Connected to the GPIO header. Not used.                        |
//| 6      | GPIO06      | Connected to the GPIO header. Not used.                        |
//| 7      | GPIO07      | Connected to the GPIO header. Not used.                        |
//| 8      | GPIO08      | Connected to the GPIO header. Not used.                        |
//| 9      | PWR_BTN_OUT | Output to the CM5 power button pin. Active low.                |
//| 10     | LED_PWR     | Power LED state from CM5. Active low.                          |
//| 11     | LED_ACTIVE  | Active LED state from CM5. Active low.                         |
//| 12     | I2C1_SDA    | I2C1 data line. CM5 is primary, the controller is secondary.   |
//| 13     | I2C1_SCL    | I2C1 clock line. CM5 is primary, the controller is secondary.  |
//| 14     | N/C         | Not connected.                                                 |
//| 15     | CM_ON       | Output from the CM5 to indicate it is powered on. Active high. |
//| 16     | TEST_MODE   | Input to enable test mode. Pull-high, active low.              |
//| 17     | GPIO17      | Connected to the test pad. Not used.                           |
//| 18     | PG_5V       | Power Good input from the 5V buck converter. Active high.      |
//| 19     | VEN         | Voltage Enable output for the 5V buck converter. Active high.  |
//| 20     | I2Cm_SDA    | I2Cm data line. Controller is primary.                         |
//| 21     | I2Cm_SCL    | I2Cm clock line. Controller is primary.                        |
//| 22     | DIS_USB3    | USB3 disable signal. Active high.                              |
//| 23     | DIS_USB2    | USB2 disable signal. Active high.                              |
//| 24     | DIS_USB1    | USB1 disable signal. Active high.                              |
//| 25     | DIS_USB0    | USB0 disable signal. Active high.                              |
//| 26     | VinS        | Analog: Scaled input voltage level.                            |
//| 27     | VscapS      | Analog: Scaled supercap voltage level.                         |
//| 28     | Iin         | Analog: Input current level.                                   |
//| 29     | GPIO29_ADC3 | Analog: ADC channel 3 input. Unused.                           |

use assign_resources::assign_resources;
use embassy_rp::peripherals;

// Version 0.3.0 pin assignments

assign_resources! {
  /// GPIO pins for the controller
  rgb_led: RGBLEDResources {
    dma_ch: DMA_CH0,
    pin: PIN_0,
    pio: PIO0,
  },
  i2cs: I2CSecondaryResources {
    sda: PIN_14,
    scl: PIN_15,
    i2c: I2C1,
  },
  i2cm: I2CPeripheralsResources {
    sda: PIN_20,
    scl: PIN_21,
    i2c: I2C0,
  },
  digital_inputs: DigitalInputResources {
    gpio05: PIN_5,
    gpio06: PIN_6,
    gpio07: PIN_7,
    gpio08: PIN_8,
    led_pwr: PIN_10,
    led_active: PIN_11,
    cm_on: PIN_13,
    pg_5v: PIN_18,
  },
  power_button_input: PowerButtonInputResources {
    pin: PIN_2,
  },
  user_button_input: UserButtonInputResources {
    pin: PIN_3,
  },
  analog_inputs: AnalogInputResources {
    adc: ADC,
    vin_s: PIN_26,
    vscap_s: PIN_27,
    iin: PIN_28,
    gpio29_adc3: PIN_29,
    temp_sensor: ADC_TEMP_SENSOR,
  },
  state_machine_outputs: StateMachineOutputResources {
    pcie_sleep: PIN_4,
    ven: PIN_19,
    dis_usb3: PIN_22,
    dis_usb2: PIN_23,
    dis_usb1: PIN_24,
    dis_usb0: PIN_25,
  },
  power_button: PowerButtonResources {
    pin: PIN_9,
  },
  test_mode: TestModeResources {
    pin: PIN_16,
  },
}

// Version 0.2.0 pin assignments

// assign_resources! {
//   /// GPIO pins for the controller
//   rgb_led: RGBLEDResources {
//     dma_ch: DMA_CH0,
//     pin: PIN_0,
//     pio: PIO0,
//   },
//   i2cs: I2CSecondaryResources {
//     sda: PIN_12,
//     scl: PIN_13,
//     i2c: I2C0,
//   },
//   i2cm: I2CPeripheralsResources {
//     sda: PIN_20,
//     scl: PIN_21,

//   },
//   digital_inputs: DigitalInputResources {
//     gpio05: PIN_5,
//     gpio06: PIN_6,
//     gpio07: PIN_7,
//     gpio08: PIN_8,
//     led_pwr: PIN_10,
//     led_active: PIN_11,
//     cm_on: PIN_15,
//     pg_5v: PIN_18,
//   },
//   power_button_input: PowerButtonInputResources {
//     pin: PIN_2,
//   },
//   user_button_input: UserButtonInputResources {
//     pin: PIN_3,
//   },
//   analog_inputs: AnalogInputResources {
//     adc: ADC,
//     vin_s: PIN_26,
//     vscap_s: PIN_27,
//     iin: PIN_28,
//     gpio29_adc3: PIN_29,
//     temp_sensor: ADC_TEMP_SENSOR,
//   },
//   state_machine_outputs: StateMachineOutputResources {
//     pcie_sleep: PIN_4,
//     ven: PIN_19,
//     dis_usb3: PIN_22,
//     dis_usb2: PIN_23,
//     dis_usb1: PIN_24,
//     dis_usb0: PIN_25,
//   },
//   power_button: PowerButtonResources {
//     pin: PIN_9,
//   },
//   test_mode: TestModeResources {
//     pin: PIN_16,
//   },
// }

// Version 0.1.0 pin assignments

// assign_resources! {
//   /// GPIO pins for the controller
//   rgb_led: RGBLEDResources {
//     dma_ch: DMA_CH0,
//     pin: PIN_4,
//     pio: PIO0,
//   },
//   i2cs: I2CSecondaryResources {
//     sda: PIN_12,
//     scl: PIN_13,
//   },
//   i2cm: I2CPeripheralsResources {
//     sda: PIN_10,
//     scl: PIN_11,
//   },
//   digital_inputs: DigitalInputResources {
//     pwr_btn_in: PIN_24,
//     user_btn: PIN_23,
//     gpio05: PIN_5,
//     gpio06: PIN_6,
//     gpio07: PIN_25,
//     gpio08: PIN_8,
//     led_pwr: PIN_18,
//     led_active: PIN_19,
//     cm_on: PIN_15,
//     pg_5v: PIN_7,
//   },
//   analog_inputs: AnalogInputResources {
//     adc: ADC,
//     vin_s: PIN_26,
//     vscap_s: PIN_27,
//     iin: PIN_28,
//     gpio29_adc3: PIN_29,
//     temp_sensor: ADC_TEMP_SENSOR,
//   },
//   state_machine_outputs: StateMachineOutputResources {
//     pcie_sleep: PIN_21,
//     ven: PIN_9,
//     dis_usb3: PIN_3,
//     dis_usb2: PIN_2,
//     dis_usb1: PIN_1,
//     dis_usb0: PIN_0,
//   },
//   power_button: PowerButtonResources {
//     pin: PIN_20,
//   },
//   test_mode: TestModeResources {
//     pin: PIN_16,
//   },
// }
