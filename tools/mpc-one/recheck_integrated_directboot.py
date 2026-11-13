#!/usr/bin/env python3
"""Re-evaluate a completed owner-private MPFree ZIP with current source logic.

Full SHA-index and CRC verification, no extraction, vendor execution, physical
USB/MaskROM, device access or mutable storage. The original ZIP is unchanged.
This is a cheap preflight before an expensive new Ghidra/QEMU research run.
"""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import stat
import zipfile


def load(local_file: str, name: str):
    where = Path(__file__).resolve().with_name(local_file)
    spec = importlib.util.spec_from_file_location(name, where)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def audit(archive: Path) -> dict:
    wide = load("synthesize_wide_research_matrix.py", "mpfree_wide_preflight")
    direct = load("synthesize_direct_boot_focus.py", "mpfree_direct_preflight")
    script = load("verify_uboot_script_wrapper.py", "mpfree_script_preflight")
    security = load("synthesize_310_security_utilities.py", "mpfree_sec_preflight")
    adc = load("synthesize_recovery_adc.py", "mpfree_adc_preflight")
    follow = load("synthesize_multitrack_followups.py", "mpfree_follow_preflight")
    with zipfile.ZipFile(archive) as z:
        members = wide.verify_zip(z)
        matrix = wide.build(z.read)
        focus = direct.build(z.read)
        stock_recovery = adc.build(z.read)
        owner_followups = follow.build(z.read)
        comparison = json.loads(
            z.read("static/wide/firmware-3.10-bootchain-v4-compare.json"))
        guest_hex = z.read("qemu-guest-boot_script_hex.txt").decode("utf-8")
        rootfs_script = script.compare_guest_to_update(
            guest_hex, comparison["boot_script"]["firmware310"])
        utilities = security.inspect_310(comparison)
    checks = {
        "fastboot_all_eight_callbacks": focus["checks"][
            "all_eight_fastboot_prefixes_exact_and_unique"],
        "reboot_modes_prefix_and_1_9": focus["checks"][
            "reboot_mode_clear_and_one_nine_branch"],
        "independent_threshold_below_100": focus["checks"][
            "separate_boot_threshold_below_100_observed"],
        "host_usb_three_distinct_families": focus["checks"][
            "all_three_host_usb_selector_families_verified"],
        "guest_script_header_matches_official": rootfs_script[
            "header_sha256_matches_official"],
        "guest_script_raw_bytes_match_official": rootfs_script[
            "raw_script_sha256_matches_official"],
        "guest_script_structure_matches_official": rootfs_script[
            "canonical_header_and_script_bytes_match_official"],
        "guest_script_commands_match_official": rootfs_script[
            "script_commands_match_official"],
        "original_adc_recovery_proven_by_arm_strings_and_fdt": (
            stock_recovery["all_existing_cross_checks_passed"]),
        "seven_track_57_question_research": (
            len(matrix["questions"])>=57 and
            len({q["track"] for q in matrix["questions"]})==7),
        "spl_order_and_mmc_label_evidence": all(
            owner_followups["spl"]["checks"].values()),
        "exact_adc_power_classification_evidence": all(
            owner_followups["power_source"]["checks"].values()),
        "updater_verify_crypto_and_callback_evidence": all(
            owner_followups["updater"]["checks"].values()),
        "loadpin_ioctl_identified": utilities["loadpin_trustpoints"][
            "ioctl_request"] == "0x40044c00",
    }
    return {
        "schema": "mpfree-offline-owner-preflight-v1",
        "archive_filename": archive.name,
        "sha256_index_entries_verified": members,
        "physical_device_access": False,
        "vendor_binary_executed": False,
        "broad_questions":len(matrix["questions"]),
        "broad_tracks":len({q["track"] for q in matrix["questions"]}),
        "focus_questions": len(focus["questions"]),
        "new_power_source_threshold_values":owner_followups[
            "power_source"]["original_threshold_words"],
        "spl_second_callback_original_word":owner_followups[
            "spl"]["second_callback_original_word"],
        "focus_topics": len(set(q["topic"] for q in focus["questions"])),
        "passed_checks": checks,
        "success": (
            all(v is True for v in checks.values())
            and len(focus["questions"]) >= 42
            and len({q["topic"] for q in focus["questions"]}) == 10
        ),
        "full_official_uboot_image_hash_available": rootfs_script[
            "original_full_image_sha256_available"],
        "boot_script_provenance_note": rootfs_script[
            "source_official_sha256_field_means"],
        "stock_adc_channel1_threshold_inclusive": stock_recovery[
            "adc_recovery_threshold_unsigned_max_inclusive"],
        "stock_adc_to_dorecovery_pointer_equal": stock_recovery[
            "recovery_flag_pointer_comparison"]["equal"],
        "stock_adc_pointer_equality_pending_new_ghidra": (
            stock_recovery["recovery_flag_pointer_comparison"]["equal"]
            is None),
        "independent_maskrom_recovery_proven": False,
    }


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--input-zip", required=True, type=Path)
    p.add_argument("--output-json", type=Path)
    a = p.parse_args()
    if (a.input_zip.is_symlink() or not a.input_zip.is_file()
            or not stat.S_ISREG(a.input_zip.lstat().st_mode)):
        p.error("Input must be an existing regular private ZIP")
    if a.output_json.exists() or a.output_json.is_symlink():
        p.error("Refusing existing report path")
    result = audit(a.input_zip)
    a.output_json.parent.mkdir(parents=True, exist_ok=True)
    a.output_json.write_text(json.dumps(result, indent=2) + "\n")
    print("SOURCE_RECHECK:", json.dumps(result, indent=2))
    if not result["success"]:
        raise SystemExit("Focused source recheck failed: do not start long run")


if __name__ == "__main__":
    main()
