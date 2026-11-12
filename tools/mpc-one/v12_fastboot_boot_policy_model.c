/* MPFree V12 synthetic fastboot/boot-policy test. PUBLIC ORIGINAL SOURCE.
 * Never runs Akai U-Boot or the updater. Never opens USB/eMMC/device nodes.
 * These assertions are safety specifications and hypothetical transitions;
 * they do NOT establish what the vendor firmware actually does.
 */
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef enum { LOCKED, AUTHORIZED } Permission;
typedef enum { INFO, DOWNLOAD, RAM_BOOT, FLASH_ROOTFS, FLASH_BOOTLOADER,
               ERASE, UNKNOWN } Operation;
typedef struct {
    Permission permission;
    uint32_t downloaded;
    unsigned resets;
    unsigned fake_writes;
} Mock;
static unsigned count = 0, failed = 0;
static void check(const char *name, bool ok) {
    printf("{\"test\":\"%s\",\"pass\":%s}\n", name, ok ? "true" : "false");
    ++count;
    if (!ok) ++failed;
}
static bool authorize(Mock *m, const char *command) {
    /* Deliberately fictitious token: not a model of vendor key verification. */
    if (!command || strcmp(command, "synthetic-owner-authorization") != 0)
        return false;
    m->permission = AUTHORIZED;
    return true;
}
static bool gate(const Mock *m, Operation op) {
    if (op == INFO) return true;
    if (op == FLASH_BOOTLOADER || op == ERASE || op == UNKNOWN) return false;
    return m->permission == AUTHORIZED;
}
static bool download(Mock *m, uint32_t size, uint32_t mock_buffer_size) {
    if (!gate(m, DOWNLOAD) || !size || size > mock_buffer_size) return false;
    m->downloaded = size;
    return true;
}
static bool mock_flash(Mock *m, const char *part) {
    if (!part || strcmp(part, "rootfs") != 0 || !gate(m, FLASH_ROOTFS))
        return false;
    ++m->fake_writes; /* Counter only; no filesystem or block IO. */
    return true;
}
typedef enum { BAD_IMAGE, LEGACY_UIMAGE, FIT_IMAGE, RAW_ZIMAGE } ImageType;
static ImageType classify(const unsigned char *b, size_t n) {
    if (!b || n < 4) return BAD_IMAGE;
    /* Classification only. None of these headers proves valid bootability. */
    if (b[0] == 0x27 && b[1] == 0x05 && b[2] == 0x19 && b[3] == 0x56)
        return LEGACY_UIMAGE;
    if (b[0] == 0xd0 && b[1] == 0x0d && b[2] == 0xfe && b[3] == 0xed)
        return FIT_IMAGE;
    if (n > 0x28 && b[0x24] == 0x18 && b[0x25] == 0x28 &&
        b[0x26] == 0x6f && b[0x27] == 0x01)
        return RAW_ZIMAGE; /* ARM zImage magic at offset 0x24 */
    return BAD_IMAGE;
}
static bool within(uint32_t start, uint32_t size, uint32_t low, uint32_t high) {
    return size && low < high && start >= low && start < high &&
           size <= high - start;
}
static bool bootcount_is_bad(uint32_t value, unsigned limit) {
    /* ONLY a synthetic candidate for a bootcount_syscon-style policy. */
    return (value >> 16) == 0xb001U && (value & 0xffffU) > limit;
}
static void reset_volatile(Mock *m) {
    m->permission = LOCKED;
    m->downloaded = 0;
    ++m->resets;
}
int main(void) {
    Mock m = {LOCKED, 0, 0, 0};
    unsigned char legacy[48] = {0x27, 0x05, 0x19, 0x56};
    unsigned char fit[48] = {0xd0, 0x0d, 0xfe, 0xed};
    unsigned char raw[48] = {0};
    raw[0x24] = 0x18; raw[0x25] = 0x28; raw[0x26] = 0x6f; raw[0x27] = 0x01;
    unsigned char invalid[48] = {0x00};

    check("locked_info_allowed", gate(&m, INFO));
    check("locked_download_denied", !gate(&m, DOWNLOAD));
    check("locked_boot_denied_model", !gate(&m, RAM_BOOT));
    check("locked_rootfs_flash_denied", !gate(&m, FLASH_ROOTFS));
    check("locked_erase_denied", !gate(&m, ERASE));
    check("unrecognized_oem_refused", !authorize(&m, "different-command"));
    check("null_oem_refused", !authorize(&m, NULL));
    check("still_locked", m.permission == LOCKED);
    check("synthetic_auth_accepted", authorize(&m, "synthetic-owner-authorization"));
    check("authorized_ram_download", gate(&m, DOWNLOAD));
    check("authorized_ram_boot_model", gate(&m, RAM_BOOT));
    check("authorized_rootfs_model", gate(&m, FLASH_ROOTFS));
    check("destructive_flash_forbidden_even_after_auth", !gate(&m, FLASH_BOOTLOADER));
    check("destructive_erase_forbidden_even_after_auth", !gate(&m, ERASE));
    check("unknown_op_forbidden", !gate(&m, UNKNOWN));
    check("zero_length_rejected", !download(&m, 0, 4096));
    check("oversized_buffer_rejected", !download(&m, 4097, 4096));
    check("fits_buffer", download(&m, 4096, 4096));
    check("buffer_count_correct", m.downloaded == 4096);
    check("rootfs_allowlisted", mock_flash(&m, "rootfs"));
    check("bootloader_partition_forbidden", !mock_flash(&m, "uboot"));
    check("environment_partition_forbidden", !mock_flash(&m, "env"));
    check("null_partition_forbidden", !mock_flash(&m, NULL));
    check("fake_write_count_only", m.fake_writes == 1);
    check("legacy_uimage_header_recognized", classify(legacy, sizeof legacy) == LEGACY_UIMAGE);
    check("fit_header_recognized", classify(fit, sizeof fit) == FIT_IMAGE);
    check("raw_zimage_header_recognized", classify(raw, sizeof raw) == RAW_ZIMAGE);
    check("invalid_header_rejected", classify(invalid, sizeof invalid) == BAD_IMAGE);
    check("short_header_rejected", classify(fit, 3) == BAD_IMAGE);
    check("null_header_rejected", classify(NULL, 0) == BAD_IMAGE);
    check("ram_bounds_ok", within(0x00800800U, 4096, 0x00800000U, 0x00900000U));
    check("ram_bounds_overflow_rejected",
          !within(0xfffffff0U, 100U, 0x00800000U, 0xffffffffU));
    check("ram_bounds_zero_rejected", !within(0x00800800U, 0, 0x00800000U, 0x00900000U));
    check("ram_outside_range_rejected", !within(0x02000000U, 4096, 0x00800000U, 0x00900000U));
    check("synthetic_bootcount_under", !bootcount_is_bad(0xb0010002U, 3));
    check("synthetic_bootcount_over", bootcount_is_bad(0xb0010004U, 3));
    check("synthetic_bootcount_wrong_magic", !bootcount_is_bad(0x11110004U, 3));
    check("synthetic_bootcount_cleared", !bootcount_is_bad(0xb0010000U, 3));
    reset_volatile(&m);
    check("reset_clears_auth", m.permission == LOCKED);
    check("reset_clears_download", m.downloaded == 0);
    check("reset_does_not_write_fake_storage", m.fake_writes == 1);
    check("reset_count", m.resets == 1);
    printf("{\"summary\":\"synthetic-only\",\"tests\":%u,\"failed\":%u,"
           "\"vendor_code_executed\":false,\"hardware_access\":false}\n", count, failed);
    puts(failed ? "MPFREE_V12_SYNTH_FAIL" : "MPFREE_V12_SYNTH_PASS");
    return failed ? 1 : 0;
}
