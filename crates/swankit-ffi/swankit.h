/* SPDX-License-Identifier: MIT OR Apache-2.0 */
#ifndef SWANKIT_INTEROP_H
#define SWANKIT_INTEROP_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
uint32_t swankit_abi_version(void);
int32_t swankit_decode_v1(uint32_t operation, const uint8_t *input,
    uint32_t input_len, uint8_t *output, uint32_t output_capacity);
#ifdef __cplusplus
}
#endif
#endif
