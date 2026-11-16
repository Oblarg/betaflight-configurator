/*
 * This file is part of Betaflight.
 *
 * Betaflight is free software. You can redistribute this software
 * and/or modify this software under the terms of the GNU General
 * Public License as published by the Free Software Foundation,
 * either version 3 of the License, or (at your option) any later
 * version.
 *
 * Betaflight is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
 *
 * See the GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public
 * License along with this software.
 *
 * If not, see <http://www.gnu.org/licenses/>.
 */

/**
 * MSP codes of the WhippyFlight firmware (`USE_WHIPPYFLIGHT`), mirroring the
 * `MSP2_WHIPPYFLIGHT_*` defines in its `msp_protocol_v2_betaflight.h` and the
 * payloads documented in `flight/whippyflight_msp.c`.
 *
 * Kept out of `MSPCodes` on purpose: `npm run check:msp-codes` holds that enum to
 * upstream firmware, which does not define these. Upstream builds answer them as
 * unsupported, which is how the WhippyFlight tab detects the firmware.
 */
enum WhippyFlightCodes {
    MSP2_WHIPPYFLIGHT_STATUS = 0x3100,
    MSP2_WHIPPYFLIGHT_UPLOAD_BEGIN = 0x3101,
    MSP2_WHIPPYFLIGHT_UPLOAD_DATA = 0x3102,
    MSP2_WHIPPYFLIGHT_UPLOAD_COMMIT = 0x3103,
    MSP2_WHIPPYFLIGHT_UPLOAD_ABORT = 0x3104,
}

/** `wf_gain_load_result_t`: why the controller has (or has not) accepted its gains. */
export enum GainLoadResult {
    OK = 0,
    TOO_SHORT = 1,
    BAD_MAGIC = 2,
    BAD_CHECKSUM = 3,
    BAD_VERSION = 4,
    RES_TOO_LARGE = 5,
    MISALIGNED = 6,
    TOO_LARGE = 7,
    NO_STORE = 8,
    BAD_LAYOUT = 9,
    LAYOUT_MISMATCH = 10,
}

/** `wf_arming_check_e`: bits of the arming checks the controller fails. */
export enum ArmingCheck {
    UPLOADING = 1 << 0,
    GAINS = 1 << 1,
    LOOP_RATE = 1 << 2,
    RPM_TELEMETRY = 1 << 3,
    SELF_LEVEL = 1 << 4,
    FAILSAFE = 1 << 5,
}

/** `wf_gain_source_t`: where the loaded gains came from. */
export enum GainSource {
    NONE = 0,
    SDCARD = 1,
    DATAFLASH = 2,
}

/** `wf_upload_result_t`. */
export enum UploadResult {
    OK = 0,
    ARMED = 1,
    NO_STORE = 2,
    TOO_LARGE = 3,
    BUSY = 4,
    NOT_STARTED = 5,
    OUT_OF_ORDER = 6,
    INCOMPLETE = 7,
    REJECTED = 8,
    VERIFY_FAILED = 9,
}

export default WhippyFlightCodes;
