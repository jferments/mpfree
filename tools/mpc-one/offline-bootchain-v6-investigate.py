#!/usr/bin/env python3
"""Offline MPFree V6 boot-chain question analysis from validated V5 reports.

Derives bounded, cross-stage evidence from previously private JSON and
Ghidra disassembly. Never executes vendor code, contacts MPC, writes boot
storage, asks for ROM mode, or attempts hardware interactions.
"""
import argparse
from collections import defaultdict, deque
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re

SHA_P1='6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f'
SHA_P3='b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a'
ELFS={
 'bootloader_signature':'/usr/bin/az01-bootloader-sig',
 'loadpin_trustpoints':'/usr/sbin/az0x-loadpin-trustpoints',
 'board_az01config':'/usr/sbin/az01config',
}
# All callsite addresses are firmware facts from prior owner reports; the
# domain labels are hypotheses until call paths and runtime are confirmed.
ANCHORS={
 'p1':{
  'rom_sram_entry':['0xff704060','0xff705208'],
  'boot_media_dispatch':['0xff705350','0xff7054a4'],
  'uimage_to_arm_handoff':['0xff705310','0xff7052c0'],
  'ddr_initialization':['0xff705994','0xff705794','0xff708e10'],
  'early_mmc':['0xff707b9c','0xff706d74','0xff7067c0'],
 },
 'p3':{
  'reboot_rom_loader':['0x00100fe4','0x001011e0'],
  'boot_media_scripts':['0x00109b68','0x00108b74'],
  'environment_storage':['0x00108804','0x00108560'],
  'fastboot_permission':['0x00122834','0x00122780'],
  'fastboot_download_and_handoff':['0x001226b4','0x00122980'],
  'image_integrity':['0x001017fc','0x0010f388','0x0010a6a4'],
  'memory_relocation':['0x00100180','0x001006e4'],
  'usb_drivers':['0x0012d018','0x0011396c','0x00113c58'],
 },
}
KEYWORDS={
 'boot_rom_and_recovery':['0xff730094','0x5242c3','0xef08a53c','recovery','dorecovery','reset_bootloader','loader'],
 'spl_media_and_handoff':['0xff705350','0xff705310','0x00100000','bootrom','rom','uimage'],
 'ddr_and_hardware':['rockchip,pctl-timing','rockchip,sdram-params','0xff610000','0xff690000'],
 'boot_selection_and_sd':['boot_targets','bootcmd','mmc1','extlinux','sysboot','scriptaddr','fdtfile'],
 'fastboot_permission':['0x0014d690','DAT_0014d690','mpf_fastboot_permission','oem','unlocked'],
 'fastboot_ram_boot':['0x00800800','0x00101744','bootm','fastboot','bootz'],
 'image_integrity_and_security':['sha256','rsa','fit','legacy','signature','secure','efuse','otp'],
 'memory_regions':['bootm_low','bootm_size','fdt_high','loadaddr','stack','reloc','ramdisk_addr_r'],
}
HOT_IMPORTS={
 'bootloader_signature':['SHA256_Init','SHA256_Update','SHA256_Final','RC4_set_key','RC4','memcmp','errx','err','fopen64','fread','fseeko64','access','printf'],
 'loadpin_trustpoints':['ioctl','open64','fdopen','__getdelim','strcmp','warn','err','write','close'],
 'board_az01config':['open64','read','write','fsync','crc32_z','strcmp','errx','strtoul','sscanf'],
}
SENSITIVE=re.compile(r'(?:password|private.?key|credential|secret|serial-number|unlock)',re.I)

def obj(folder,name):
    path=folder/name
    if not path.is_file() or path.is_symlink():
        raise FileNotFoundError(str(path))
    return json.loads(path.read_text(encoding='utf-8'))

def all_stage(folder,kind):
    v3=obj(folder,f'{kind}-evidence-graph-v3.json')
    v4=obj(folder,f'{kind}-coverage-v4.json')
    expected=SHA_P1 if kind=='p1' else SHA_P3
    if v3.get('sha256')!=expected or v4.get('source_payload_sha256')!=expected:
        raise ValueError(f'{kind} image provenance mismatch')
    graph=v3['all_known_function_graph']
    detailed={**v3['detailed_functions'],**v4['coverage']}
    return v3,graph,detailed

def near(graph,roots,steps=2):
    rev=defaultdict(set)
    for src,meta in graph.items():
        for dst in meta.get('calls',[]):rev[dst].add(src)
    result={}; queue=deque((root,0) for root in roots if root in graph)
    while queue and len(result)<250:
        node,depth=queue.popleft()
        if node in result:continue
        result[node]=depth
        if depth<steps:
            for nxt in list(graph[node].get('calls',[]))[:40]+sorted(rev[node])[:40]:
                if nxt not in result and nxt in graph:queue.append((nxt,depth+1))
    return result

def stage_investigation(folder,stage):
    v3,graph,detailed=all_stage(folder,stage)
    rev=defaultdict(set); computed=[]; decompile_failures=[]
    for k,meta in graph.items():
        for v in meta.get('calls',[]):rev[v].add(k)
        if meta.get('computed_calls'):
            computed.append({'function':k,'name':meta.get('name'),
                             'callsites':meta['computed_calls'],
                             'number_of_callers':len(rev.get(k,[]))})
    # Compute caller counts only once all edges have been seen.
    for item in computed:item['number_of_callers']=len(rev.get(item['function'],[]))
    for addr,fn in detailed.items():
        if fn.get('decompile_error') or fn.get('error'):
            decompile_failures.append({'address':addr,'error':str(fn.get('decompile_error') or fn.get('error'))[:500]})
    frontier={}
    for group,anchors in ANCHORS[stage].items():
        nodes=near(graph,anchors,steps=2)
        frontier[group]={
            'seed_addresses':anchors,
            'seed_functions_recognized':[x for x in anchors if x in graph],
            'nearby_function_count':len(nodes),
            'nearby_functions':[{'entry':k,'name':graph[k].get('name'),
                                 'hop_distance':v,
                                 'computed_calls':graph[k].get('computed_calls',[])[:8],
                                 'decompiled':bool(detailed.get(k,{}).get('pseudocode'))}
                                for k,v in sorted(nodes.items(),key=lambda x:(x[1],x[0]))][:100],
        }
    kw={}
    for domain,words in KEYWORDS.items():
        matches=[]
        for addr,row in detailed.items():
            src=row.get('pseudocode','')
            found=[w for w in words if w.lower() in src.lower()]
            if not found:continue
            matches.append({'address':addr,'function':row.get('name'),
                            'matched_words':found,'called_by':sorted(rev.get(addr,[]))[:20],
                            'direct_callees':graph.get(addr,{}).get('calls',[])[:20]})
        kw[domain]={'total_matching_functions':len(matches),'functions':matches[:150]}
    roots=set(graph)
    for meta in graph.values():
        roots.difference_update(dst for dst in meta.get('calls',[]) if dst in graph)
    computed.sort(key=lambda a:(-len(a['callsites']),-a['number_of_callers'],a['function']))
    return {'recognized':len(graph),'detailed_entries':len(detailed),
            'successful_pseudocode':sum(bool(row.get('pseudocode')) for row in detailed.values()),
            'decompile_failures':decompile_failures,
            'all_direct_call_edges':sum(len(v.get('calls',[])) for v in graph.values()),
            'all_indirect_call_sites':sum(len(v.get('computed_calls',[])) for v in graph.values()),
            'functions_without_detected_direct_callers':len(roots),
            'possible_roots_not_proven_runtime_starts':sorted(roots)[:90],
            'callgraph_frontiers':frontier,
            'keyword_evidence':kw,
            'ranked_indirect_functions':computed[:70],
            'method_limits':[
                'Zero direct-call predecessors is NOT proof of startup reachability.',
                'Caller/callee expansions include non-executed generic library code.',
                'Ghidra function-boundary and ARM/Thumb mode inference may be wrong.',
                'No MMIO register values or runtime global initialization observed.',
            ]}

INSN=re.compile(r'^\s*([0-9a-fA-F]+):\s*([A-Za-z][A-Za-z0-9.]*)\s+(.*)$')
CALL=re.compile(r'\b(?:bl|blx|bx)\b')
TGT=re.compile(r'\b(?:bl|blx)\s+([0-9a-fA-F]+)\s*<([^>]+)>')

def classify_elf(code,import_names):
    insns=[]
    for ix,line in enumerate(code):
        match=INSN.match(line)
        if not match:continue
        addr=int(match.group(1),16)
        insns.append({'addr':addr,'at':f'0x{addr:x}',
                      'op':match.group(2),'arguments':match.group(3),
                      'source':line[:220], 'line':ix})
    hotspots=[]
    wanted=set(import_names)
    for i,ins in enumerate(insns):
        if ins['op'] not in ('bl','blx','bx'):continue
        target=TGT.search(ins['source'])
        if not target:continue
        callee=target.group(2).split('@')[0]
        if not any(s.lower() in callee.lower() for s in wanted):continue
        before=insns[max(0,i-9):i]
        after=insns[i+1:i+8]
        hotspots.append({'address':ins['at'],'callee':callee,
                         'address_range':[f'0x{before[0]["addr"]:x}' if before else ins['at'],
                                          f'0x{after[-1]["addr"]:x}' if after else ins['at']],
                         'nearby_instructions':[x['source'] for x in before[-9:]+[ins]+after[:7]]})
    return {'number_disassembly_instructions':len(insns),
            'callsite_candidates':hotspots[:180],
            'actual_imported_call_count':len(hotspots),
            'limitations':["Nearby instructions do not prove execution or full argument provenance.",
                          "ARM/Thumb disassembly may interleave literal pools with instructions.",
                          "Assembly source originates from prior privately verified ELF artifact."]}

def elf_investigation(folder):
    v4=obj(folder,'firmware-3.10-bootchain-v4-compare.json')
    results={}
    for name, path in ELFS.items():
        v5=obj(folder,'v5-ghidra-'+name+'.json')
        from_v4=v4['elfs'].get(path,{}).get('firmware310',{})
        if v5.get('source_sha256_verified_by_launcher')!=from_v4.get('sha256'):
            raise ValueError('ELF provenance inconsistent for '+name)
        script=from_v4.get('small_elf_complete_text',{})
        disasm=classify_elf(script.get('lines',[]),HOT_IMPORTS[name])
        recognized=v5.get('functions',{})
        useful=[{'entry':k,'name':v.get('name'),
                 'has_pseudocode':bool(v.get('pseudocode')),
                 'bytes_of_pseudocode':len(v.get('pseudocode',''))}
                for k,v in recognized.items() if v.get('body_size',0)>20]
        references=[{'text':('[REDACTED]' if SENSITIVE.search(s.get('text','')) else s.get('text')),
                     'address':s.get('address'),'xref_count':len(s.get('references',[]))}
                    for s in v5.get('selected_strings',[]) if len(s.get('text',''))>4]
        results[name]={
           'elf':path,'sha256':from_v4.get('sha256'),'bytes':from_v4.get('bytes'),
           'ghidra_recognized_functions':len(recognized),
           'ghidra_import_stub_count':sum(v.get('body_size',0)<=16 for v in recognized.values()),
           'functions_over_20bytes':useful,
           'selected_data_strings':references,
           'number_strings_without_refs':sum(len(x.get('references',[]))==0 for x in v5.get('selected_strings',[])),
           'raw_arm_disassembly':disasm,
           'next_step':"Recover main from ELF dynamic relocation/entry and use Ghidra to reconstruct omitted .text functions and arguments; never execute the binary.",
        }
    return results

def firmware_investigation(folder):
    f=obj(folder,'firmware-3.10-bootchain-v4-compare.json')
    security={}
    for name,data in f.get('elfs',{}).items():
        a=data.get('factory',{});b=data.get('firmware310',{})
        if not a.get('present') and not b.get('present'):continue
        security[name]={'factory_present':a.get('present'),
            'firmware310_present':b.get('present'),
            'hash_identical':a.get('sha256')==b.get('sha256') if a.get('sha256') and b.get('sha256') else None,
            'factory_sha':a.get('sha256'),'firmware310_sha':b.get('sha256')}
    scripts=f.get('boot_script',{})
    trees={}
    for path,item in f.get('board_dtbs',{}).items():
        diff=item.get('boot_node_diff',{})
        trees[path]={'changed_selected_node_count':diff.get('nodes_changed'),
                     'selected_nodes':list(diff.get('changed_boot_nodes',{}).keys()),
                     'reboot_mode_loader_alias':'mode-update = <0x5242c301>' if any('reboot-mode' in x for x in diff.get('changed_boot_nodes',{})) else None}
    services={path:{k:{'present':v.get('present'),'sha256':v.get('sha256')}
                    for k,v in versions.items()}
              for path,versions in f.get('services',{}).items()}
    return {'boot_scripts_normalized_equal':scripts.get('normalized_commands_equal'),
            'board_dtb_selected_diffs':trees,
            'binary_presence_and_changes':security,
            'service_presence':services,
            'runtime_enforcement_unproven':True,
            'security_layers':['Silicon ROM/eFuse','SRAM SPL','U-Boot','Linux kernel',
                               'userspace services / update utilities']}

def questions():
    return [
    # ROM, hardware recovery
    ('Q111','ROM','Does an externally accessible USB connector route to silicon USB OTG when the CPU is still in BootROM?', 'Physical USB connection and enumeration; static node alone insufficient','hardware'),
    ('Q112','ROM','Can true MaskROM be entered while the eMMC ID block remains valid?', 'ROM protocol enumeration vs U-Boot Fastboot state','hardware'),
    ('Q113','ROM','Does CONFIG_SPL_ROCKCHIP_BACK_TO_BROM return only to normal media loading rather than USB download?', 'Compare Rockchip docs and SPL control flow with device-specific evidence','static + hardware'),
    ('Q114','ROM','Which exact ROM boot-media priority applies to this SoC and MPC PCB?', 'RK3288 TRM boot flow vs board media wiring','static + hardware'),
    ('Q115','ROM','Can we make a physically independent eMMC restore pathway WITHOUT first disabling the factory loader?', 'Non-destructive independent readback/restore demonstration on spare hardware; current device remains intact','hardware'),
    ('Q116','ROM','Are eMMC alternate boot partitions or boot-select fuses involved in this MPC One?', 'Read-only EXT_CSD and boot partition configuration, no programming','read only runtime'),
    # SPL
    ('Q117','SPL','What is the exact BootROM first instruction and ARM/Thumb state of SRAM stage?', 'P1 raw entry plus ARM/Thumb decode and stack trace','static'),
    ('Q118','SPL','Does SPL return to ROM, reset CPU, or hand off via descriptor callback?', 'P1 callback target data-flow and actual branch target','static + runtime'),
    ('Q119','SPL','Which of five SPL boot-media candidates map to actual MMC and USB controllers?', 'Device table pointer resolution and cross-stage FDT','static'),
    ('Q120','SPL','Can a custom SRAM SPL initialize DDR using exact owner P1/FDT geometry?', 'Reproduce implementation offline first; only hardware RAM test after independent rescue','hardware gated'),
    ('Q121','SPL','Does owner SPL enforce cryptographic signatures before U-Boot?', 'Trace full image check and failure branch, not CRC existence','static'),
    # Boot policy
    ('Q122','Boot policy','Is U-Boot SDMMC a fallback only or can it be selected without on-device writes?', 'Resolve environment evaluation and input/console command reachability','static + runtime'),
    ('Q123','Boot policy','Does U-Boot console unlock on keypress despite bootdelay=0?', 'Read-only UART RX capture before attempting input','hardware'),
    ('Q124','Boot policy','Can extlinux.conf on SD be reached when eMMC stock boot script succeeds?', 'Trace distro boot control flow and success return conditions','static'),
    ('Q125','Boot policy','Can the existing Linux kexec environment select SD U-Boot as a one-time reboot?', 'Board-specific PMU mode handling; no inferred command until validated','static + runtime'),
    ('Q126','Boot policy','Does U-Boot console support a temporary bootcmd override in RAM?', 'Read-only console capture and command table; no saveenv','runtime gated'),
    # Fastboot
    ('Q127','Fastboot','Which OEM command sets permission variable at 0x0014d690?', 'Trace dispatcher, compare opcode and caller state in validated P3 pseudocode','static'),
    ('Q128','Fastboot','Are getvar and other informational commands gated separately from download/flash?', 'Model Fastboot handler registration and checks','static'),
    ('Q129','Fastboot','Which image types does Fastboot boot actually accept, and what return/reset follows bad input?', 'Bootm/image type/verify paths; no malformed live tests','static'),
    ('Q130','Fastboot','Can RAM boot avoid every write to eMMC and SD at kernel, initramfs, script and firmware layers?', 'Audit initramfs filesystem mounts, MMC driver permissions and rescue procedure','static + hardware'),
    ('Q131','Fastboot','What is the bounded safe RAM download capacity after U-Boot relocation?', 'Memory relocation/stack/FDT/heap analysis; reject oversized protocol max','static + runtime'),
    # Userland binaries
    ('Q132','Userspace safety','Does az01config program SoC eFuses or a software named fuse record?', 'Extract main and command dispatch data from static ELF; never invoke program-fuses','static'),
    ('Q133','Userspace safety','Which write-capable subcommands of az01config target partitions 3/4/11?', 'Resolve dispatch table and libc open/write/fsync call arguments','static'),
    ('Q134','Userspace safety','What 32-byte digest inputs and RC4 transformation are used by bootloader-sig?', 'Full program main and heap/data pointer tracing; do not run binary','static'),
    ('Q135','Userspace safety','Is bootloader-sig invoked at every boot or only by OTA/update flows?', 'Search systemd units, executable callers, updater RAMFS scripts','static + runtime'),
    ('Q136','Userspace safety','Is a signature failure fatal for boot, updater, or only this CLI?', 'Trace process invocation order and unit dependencies','static + runtime'),
    ('Q137','Userspace safety','What is the exact ioctl to /sys/kernel/security/loadpin/dm-verity?', 'Decode helper main, fd arguments, kernel ABI and unit ordering','static'),
    ('Q138','Userspace safety','Are LoadPin constraints scoped to kernel rootfs or also independent MPFree kernels?', 'Identify who enables LSM policy in boot chain','static + runtime'),
    ('Q139','Userspace safety','Does the newer updater have a reachable early bootloader write code path on rootfs-only update?', 'Compare 3.6/3.10 libaz01-prog call graph, update PART conditions','static'),
    # Upgrade resilience
    ('Q140','Upgrade resilience','Can future firmware modify eMMC P1/P3, environment or fallback loader slots?', 'Inventory updater writes and partition layout in both versions','static'),
    ('Q141','Upgrade resilience','Can recovery remain independent of both official Linux and the installed U-Boot?', 'Verified BootROM USB + temporary loader + readback + rehearsed restore','hardware gated'),
    ('Q142','Upgrade resilience','Would SD-first need silicon media priority override or only one-shot U-Boot input?', 'Distinguish ROM, SPL, U-Boot selection levels','static + hardware'),
    ('Q143','Upgrade resilience','Is factory eMMC bootloader capable of independently booting MPFree kernel/FDT from SD?', 'Prove bootz load addresses and FDT ABI; safe staged runtime test','static + hardware'),
    ('Q144','Upgrade resilience','Can official firmware updates remove kexec without impacting the independent boot choice?', 'Specify and prove boot path containing no Akai kernel/userspace','architecture + runtime'),
    # Risk gates
    ('Q145','Safety','Can we guarantee 100% recovery from arbitrary power, NAND wear, physical damage, or secure fuse writes?', 'No; do not claim absolute guarantees. Use explicit fault model and independent recovery evidence','not guaranteed'),
    ('Q146','Safety','Which runtime queries are observational and never intentionally touch persistent boot state?', 'SSH cat/proc/sys, host USB listing and RX-only UART, no reset/mount/write','runtime read only'),
    ('Q147','Safety','What proof gates are needed before loading even a RAM-only kernel?', 'Independent recovery, safe memory bounds, no MMC writes, watchdog, logs, dry-run','hardware gated'),
    ('Q148','Safety','How will we prove a full restore from an independently loaded tool without touching factory boot partitions?', 'Separate sacrificial test storage/board before any production restoration','hardware gated'),
    ]

def make(folder):
    result={'schema':'mpfree-v6-cross-layer-evidence-v1',
            'created_utc':datetime.now(timezone.utc).isoformat(),
            'source':'owner-private verified V3/V4/V5 reports; no live test',
            'boot_stages':{x:stage_investigation(folder,x) for x in ('p1','p3')},
            'linux_helpers':elf_investigation(folder),
            'firmware_compatibility':firmware_investigation(folder),
            'questions':[{'id':a,'domain':b,'question':c,'strategy':d,'required_evidence':e} for a,b,c,d,e in questions()],
            'hard_stops':[
              'No 100% guarantee for a physical computing device or arbitrary failure conditions.',
              'Never execute az01config, program-fuses, vendor updater, or unknown write-capable CLI.',
              'Do not disable a working eMMC bootloader to force MaskROM.',
              'Do not write eMMC P1/P2/P3 or eFuses before an independently validated recovery mechanism.',
              'Do not equate U-Boot loader/Fastboot with genuine silicon MaskROM.',
              'Do not mistake compile-time codepaths and absence of XREFs for runtime behavior.',
              'Do not infer direct SD boot from MMC1 support alone.',
            ]}
    return result

def summary(r):
    lines=['# MPFree V6 private cross-layer evidence report','',
           '> Offline only. No physical experiment was performed. All ELF and Ghidra evidence is private.','',
           '## What this pass actually covered','']
    for name,stage in r['boot_stages'].items():
        lines+=['### '+name.upper(),'',
               f"- Recognized Ghidra functions: **{stage['recognized']}**; containing pseudocode: **{stage['successful_pseudocode']}**.",
               f"- Direct edges: **{stage['all_direct_call_edges']}**; indirect call sites: **{stage['all_indirect_call_sites']}**.",
               f"- Decompilation failures: {len(stage['decompile_failures'])}.",'']
    lines+=['## Linux programs: important caution','']
    for name,e in r['linux_helpers'].items():
        lines+=[f"- {name}: {e['ghidra_recognized_functions']} Ghidra functions but {e['ghidra_import_stub_count']} tiny import stubs; "
                f"{e['raw_arm_disassembly']['actual_imported_call_count']} relevant assembly callsite candidates in earlier ELF disassembly."]
    lines+=['','The userspace ELF imports did not fully recover all stripped main functions.',
            'A name such as program-fuses identifies a potentially destructive interface; do not execute the binary to learn more.',
            '', '## Open questions catalog','']
    bygroup=defaultdict(list)
    for q in r['questions']:bygroup[q['domain']].append(q)
    for group,qs in bygroup.items():
        lines.extend([f'### {group}',''])
        for q in qs:
            lines.append(f"- **{q['id']}** {q['question']} — evidence: {q['required_evidence']}.")
        lines.append('')
    lines+=['## Runtime recoverability','',
            '**100% recovery cannot be honestly certified.** Read-only observations should be completed first.',
            'Genuine ROM entry and complete independent recovery must be separately established.',
            'RAM-only boot is lower risk than flashing, but can still write storage if wrong kernel, scripts, or drivers run.',
            'The next stage should require a formal pass/fail preflight and a stop-on-uncertainty rule.', '']
    return '\n'.join(lines)

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('report_dir',type=Path)
    a=p.parse_args()
    root=a.report_dir.expanduser()
    if not root.is_dir() or root.is_symlink() or not root.name.startswith('bootchain-wide-'):
        p.error('Require owner-private bootchain-wide-* report directory')
    result=make(root)
    old=os.umask(0o077)
    try:
        (root/'v6-static-evidence.json').write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
        (root/'v6-static-summary.md').write_text(summary(result),encoding='utf-8')
    finally:os.umask(old)
    print('V6 complete:',len(result['questions']),'explicit new questions;',
          sum(x['all_indirect_call_sites'] for x in result['boot_stages'].values()),'indirect sites')
if __name__=='__main__':main()