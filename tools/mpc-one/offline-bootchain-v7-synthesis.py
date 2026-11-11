#!/usr/bin/env python3
"""V7 evidence ledger: joins verified V6 and targeted V7 offline results.

Separates confirmed static code, plausible inferences, missing observations
and *independently verified hardware* requirements for owner-controlled boot.
Generated owner-private JSON and Markdown, never executes firmware.
"""
import argparse
import json
import os
from pathlib import Path

CATALOG={
 'Q149':('Linux ELF dispatch','Which five az01config command names map to which handlers?','ELF relocation-backed command table and handler addresses'),
 'Q150':('Fuse safety','Does program-fuses modify SoC OTP, eMMC EXT_CSD or software records?','Handler code and side-effect calls; no execution'),
 'Q151':('Fuse safety','Can az01config program fuses indirectly through device nodes or ioctls?','Static handler syscall/ioctl arguments and referenced paths'),
 'Q152':('Fuse safety','Which configuration commands write which partitions or hardware registers?','Command table -> handler -> open/write/fsync call graph'),
 'Q153':('Loader signature','Which exact partitions/byte ranges feed each SHA256 digest?','ELF main, SHA wrapper functions, path-to-fd propagation'),
 'Q154':('Loader signature','Where does RC4 operate and what are the key/digest derivation steps?','Main/compute helpers, input-output alias and transformation order'),
 'Q155':('Loader signature','Is the digest check invoked on every normal boot?','Bounded binary+systemd references, then optional runtime process observation'),
 'Q156':('Loader signature','Is the digest comparison fatal to device boot or only a tool invocation?','Caller chain and service failure semantics'),
 'Q157':('LoadPin','Is ioctl 0x40044c00 LOADPIN_IOC_SET_TRUSTED_VERITY_DIGESTS?','Cross-match Linux uapi loadpin.h and Ghidra dataflow'),
 'Q158':('LoadPin','Which digest file is passed to the kernel and when is it pinned?','Recovered main, service ordering, kernel source'),
 'Q159':('LoadPin','Which errno fallback paths permit recovery without a digest list?','Main control flow and kernel return codes'),
 'Q160':('LoadPin','Could changing firmware disable kexec through Linux LoadPin policy?','Kernel config, enforcement parameter and verified runtime state'),
 'Q161':('Firmware update','Does 3.10 userspace ever invoke az01config or bootloader-sig?','Bounded binary reference scan and offline service inventory'),
 'Q162':('Firmware update','Can future official updates rewrite SPL, U-Boot or eMMC boot areas?','Updater function call chain and update container partition rules'),
 'Q163':('Firmware update','Is bootloader-sig an update prerequisite or maintenance-only check?','Invoker identification and branch/failure handling'),
 'Q164':('ROM','Does the MPC expose true ROM-mode USB independently of all installed firmware?','Hardware: VID/PID observed in genuine MaskROM state'),
 'Q165':('ROM','Which exact physical connector is connected to RK3288 USB OTG in ROM mode?','Board wiring measurement and passive enumeration'),
 'Q166':('ROM','Do eMMC boot0, boot1, RPMB, EXT_CSD and secure OTP alter recovery?','Read-only complete storage/fuse inventory (no OTP write)'),
 'Q167':('ROM','What non-destructive test proves independent eMMC recovery?','External recovery loader readback and sacrificial device restore rehearsal'),
 'Q168':('SPL','Which SPL callbacks handle storage selection and final U-Boot handoff?','Resolve indirect function pointers + observed UART timing'),
 'Q169':('SPL','Can an owner-built loader reproduce DDR init without corrupting factory SPL?','Independent SRAM RAM-only loader on verified recovery environment'),
 'Q170':('SPL','Does hardware policy require a signed first-stage image?','True ROM SRAM owner code acceptance; static not sufficient'),
 'Q171':('Boot selection','Can stock U-Boot deliberately boot SD without editing eMMC environment?','Read-only UART/USB/ADC observation, authorized transient boot choice'),
 'Q172':('Boot selection','Does factory P3 support USB host boot and SD extlinux when eMMC works?','Command registration and runtime boot-target reachability'),
 'Q173':('Boot selection','Can future Akai firmware retain factory SPL but replace U-Boot?','Compare update partition records and persistent P3 hashes over versions'),
 'Q174':('Boot selection','What owner-controlled path survives an Akai kernel with disabled kexec?','Direct U-Boot boot independent of Akai Linux'),
 'Q175':('Fastboot','What exact command changes Fastboot authorization state?','Permission global data-flow and OEM dispatcher table'),
 'Q176':('Fastboot','What are the real RAM buffer/stack/relocation bounds for Fastboot boot?','U-Boot runtime relocation + read-only memory map'),
 'Q177':('Fastboot','Does failed RAM boot reliably return or trigger reset?','Static bootm return/reset path; verified recovery required first'),
 'Q178':('Recovery','Can we guarantee recovery from arbitrary hardware failure or OTP changes?','No; define recoverable software fault model only'),
 'Q179':('Recovery','Have we proved backup coverage of eMMC user area and hardware boot partitions?','Read-only backup/geometry and independent restore verification'),
 'Q180':('Recovery','What is the minimum independent-recovery gate before modifying boot storage?','Physical ROM entry, known-good loader, complete readback and test restore'),
}

def read(root,name):
    p=root/name
    if not p.is_file() or p.is_symlink():return None
    try:return json.loads(p.read_text())
    except (OSError,ValueError):return None

def build(root):
    v6=read(root,'v6-static-evidence.json') or {}
    ghidra=[read(root,'v6-elf-'+n+'-main-recovery.json') for n in
            ('bootloader_signature','loadpin_trustpoints','board_az01config')]
    startup=read(root,'v6-startup-usage.json') or {}
    dispatch=read(root,'v7-elf-dispatch.json') or {}
    unwind=read(root,'v7-arm-unwind.json') or {}
    scan=read(root,'v7-binary-references.json') or {}
    verified={'v6':v6.get('schema')=='mpfree-v6-cross-layer-evidence-v1',
              'all_elf_recoveries':all(x and x.get('schema')=='mpfree-v6-relocation-function-recovery' for x in ghidra),
              'dispatch':dispatch.get('schema')=='mpfree-v7-elf-command-dispatch-v1',
              'unwind':unwind.get('schema')=='mpfree-v7-arm-exidx-function-starts-v1',
              'binary_references':scan.get('schema')=='mpfree-v7-offline-binary-reference-scan-v1'}
    safety=[
        {'gate':'Independent ROM-mode interface','state':'NOT VERIFIED','reason':'No live silicon ROM enumeration in ZIP'},
        {'gate':'True full eMMC restore path','state':'NOT VERIFIED','reason':'Existing user-area backup alone cannot prove independent restore'},
        {'gate':'Temporary SD-first boot route','state':'NOT VERIFIED','reason':'mmc0 precedes mmc1 and eMMC boot is successful'},
        {'gate':'Fastboot RAM boot bounds','state':'NOT VERIFIED','reason':'Protocol max length does not establish safe available DRAM'},
        {'gate':'Owner SPL accepted in RAM','state':'NOT VERIFIED','reason':'No physical SRAM owner-code probe'},
        {'gate':'100 percent recoverability','state':'IMPOSSIBLE TO GUARANTEE','reason':'Hardware faults, OTP writes, wear, damage outside software fault model'},
    ]
    evidence={
      'processed_stages':v6.get('boot_stages',{}),
      'elf_dispatch_tables':dispatch.get('candidate_tables',[]),
      'fuse_handler_candidates':dispatch.get('program_fuses_mapping',[]),
      'unwind_function_counts':{k:len(v.get('candidate_functions',[])) for k,v in unwind.get('executables',{}).items()},
      'binary_reference_scan':{'scanned_files':scan.get('scanned_file_count',0),
            'matching_files':len(scan.get('references',[])),
            'warnings':scan.get('limitations',[])},
      'startup_text_scan':startup.get('statistics',{}),
      'ghidra_new_main_recovery':{name: {'candidate_count':len(x.get('candidates',[])),
                                   'decompiled':x.get('successfully_decompiled',0)}
              for name,x in zip(('bootloader_signature','loadpin_trustpoints','board_az01config'),ghidra) if x},
    }
    out={'schema':'mpfree-v7-cross-layer-closure-v1',
         'input_verification':verified,'evidence':evidence,
         'new_questions':[{'id':k,'domain':v[0],'question':v[1],'next_evidence':v[2]}
                          for k,v in CATALOG.items()],
         'live_testing':{'performed':False,'automatic_device_access':False,
                         'allowed':'offline filesystem and Ghidra data only'},
         'recovery_gates':safety,
         'interpretation_limits':[
           'No matched binary string is not proof the utility is never invoked.',
           'A relocation-backed function pointer is not a proven executed command.',
           'No confidence in hardware recoverability until ROM entry and restore are demonstrated.',
         ]}
    return out

def summary(data):
    v=data['evidence'];lines=['# MPFree V7 owner-private evidence summary','',
        'Only offline results. No MPC operation was performed.','',
        '## V6 baseline and V7 improvements','',
        f'- V6 prior report schema valid: {data["input_verification"]["v6"]}.',
        f'- Recovered ELF main reports all present: {data["input_verification"]["all_elf_recoveries"]}.',
        f'- Command table candidates: {len(v["elf_dispatch_tables"])}.',
        f'- Identified program-fuses handler candidates: {len(v["fuse_handler_candidates"])}.',
        f'- Offline binary/reference scan: {v["binary_reference_scan"]["scanned_files"]} files.',
        '', '## Active research questions', '',
        f'Q149–Q180: {len(data["new_questions"])} new questions in v7-evidence.json.',
        '', '## Critical hardware recovery gates', '']
    for g in data['recovery_gates']:
        lines.append(f'- **{g["gate"]}**: {g["state"]} — {g["reason"]}.')
    lines+=['','The private JSON contains provenance, raw table addresses and call candidates.',
            'Do not publish owner firmware or Ghidra-derived proprietary disassembly.','']
    return '\n'.join(lines)

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('report_dir',type=Path)
    a=p.parse_args();root=a.report_dir
    if not root.is_dir() or root.is_symlink() or not root.name.startswith('bootchain-wide-'):
        p.error('Require report directory')
    data=build(root)
    old=os.umask(0o077)
    try:
        (root/'v7-evidence.json').write_text(json.dumps(data,indent=2)+'\n')
        (root/'v7-summary.md').write_text(summary(data))
    finally:os.umask(old)
    print('V7 evidence summary complete, new questions',len(data['new_questions']))
if __name__=='__main__':main()