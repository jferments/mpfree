# @runtime PyGhidra
# @category MPFree
# Focused, read-only Rockchip SPL/U-Boot and Akai updater analysis.
# Executable sources are SHA-checked by the owner-side launcher, not executed.
import json
import os
import re
from pathlib import Path
from datetime import datetime, timezone
from collections import defaultdict, deque
from ghidra.app.decompiler import DecompInterface

args=list(getScriptArgs())
if len(args)!=4:
    raise RuntimeError('Expected role output_json source_sha256 max_functions')
role=str(args[0]); destination=Path(str(args[1])); digest=str(args[2]); limit=int(args[3])
if role not in ('spl','uboot','az01-update','libaz01-prog','ramfs-updater'):
    raise RuntimeError('Unsupported audit role')
if not destination.is_absolute() or not re.fullmatch(r'[0-9a-f]{64}',digest) or not 1<=limit<=500:
    raise RuntimeError('Unsafe arguments')
program=currentProgram
if program is None or 'ARM' not in str(program.getLanguageID()):
    raise RuntimeError('Expected ARM executable')
# Recheck the binary that Ghidra loaded, rather than trusting that the
# previously saved project is necessarily still the one backed up on disk.
if role in ('spl','uboot'):
    import hashlib
    from jpype import JArray, JByte
    expected_size=32768 if role=='spl' else 363649
    expected_base=0xff704000 if role=='spl' else 0x00100000
    a=program.getAddressFactory().getDefaultAddressSpace().getAddress(expected_base)
    block=program.getMemory().getBlock(a)
    if not block or block.getStart().getOffset()!=expected_base or block.getSize()!=expected_size:
        raise RuntimeError('Mismatched existing Ghidra stage memory block')
    buf=JArray(JByte)(32768); h=hashlib.sha256()
    for off in range(0,expected_size,32768):
        n=min(32768,expected_size-off)
        target=program.getAddressFactory().getDefaultAddressSpace().getAddress(expected_base+off)
        if int(program.getMemory().getBytes(target,buf,0,n))!=n:
            raise RuntimeError('Unreadable stage memory bytes')
        h.update(bytes(int(v)&255 for v in buf[:n]))
    if h.hexdigest()!=digest:
        raise RuntimeError('Project SHA differs from exact verified owner SPL/U-Boot stage')
fm=program.getFunctionManager(); refs=program.getReferenceManager(); listing=program.getListing(); memory=program.getMemory()
base=program.getImageBase().getOffset()
patterns={
 'rom_mode':r'bootrom|brom|maskrom|rockusb|reboot|loader|rom|0xef08a53c|0x5242c3',
 'sram_spl':r'spl|ddr|dram|uimage|boot_device|mmc|bootrom|loadaddr|go|jump',
 'image_integrity':r'sign|verif|digest|sha256|rsa|crc|header|hash|check|rollback',
 'update_dispatch':r'az01_update|image|partition|dispatch|program|command|recovery|bootloader|upgrade',
 'writes':r'write|erase|flash|fsync|blkdiscard|mtd|emmc|mmcblk|fuse|otp|bootpart',
 'usb_fastboot':r'fastboot|usb|download|bootm|bootz|getvar|oem',
 'environment':r'fw_setenv|env|bootcmd|boot_targets|dorecovery|mmc0|mmc1',
 'system':r'kexec|system\(|exec|fork|reboot|poweroff|mount|rootfs|initramfs',
}
compiled={k:re.compile(v,re.I) for k,v in patterns.items()}
sensitive=re.compile(r'secret|password|private.?key|token|credentials|serial.?number|ethaddr',re.I)
# These addresses refer to the exact SHA-verified user's static SPL/U-Boot
# projects. They are evidence anchors, not runtime executable claims.
seeds={
 'spl':[0xff704000,0xff704020,0xff705208,0xff705260,0xff705274,
        0xff7052c0,0xff705310,0xff705350,0xff7054a4],
 'uboot':[0x00100fe4,0x001011e0,0x00101744,0x001017fc,
          0x00122834,0x00122980,0x001226b4,0x00122bf8,0x00122c5c,
          0x0010f388]
}
functions={}
it=fm.getFunctions(True)
while it.hasNext():
    f=it.next()
    functions[str(f.getEntryPoint())]=f

# Build call relationships from Ghidra references, not guessed symbol names.
callers=defaultdict(set); callees=defaultdict(set); indirect=defaultdict(list)
for address, fn in functions.items():
    instruction=listing.getInstructions(fn.getBody(),True)
    while instruction.hasNext():
        ins=instruction.next()
        typ=ins.getFlowType()
        if not typ.isCall():continue
        targets=[str(t) for t in ins.getFlows()]
        if not targets:
            if len(indirect[address])<60:indirect[address].append(str(ins.getAddress()))
        else:
            for target in targets:
                callees[address].add(target)
                callers[target].add(address)

# Defined strings are collected to find update/bootloader xrefs. Ghidra's
# importer may miss some strings; these are leads, not a complete index.
string_hits=[]; scores=defaultdict(int); groups=defaultdict(set)
listing_data=listing.getDefinedData(True)
while listing_data.hasNext() and len(string_hits)<1100:
    d=listing_data.next()
    try:
        value=d.getValue()
        if value is None:continue
        text_value=str(value)
        if not 4<=len(text_value)<=600:continue
        matched=[name for name,exp in compiled.items() if exp.search(text_value)]
        if not matched:continue
        if sensitive.search(text_value):text_value='[REDACTED POTENTIALLY SENSITIVE STRING]'
        xrefs=[]
        for x in refs.getReferencesTo(d.getAddress()):
            parent=fm.getFunctionContaining(x.getFromAddress())
            if parent is None:continue
            key=str(parent.getEntryPoint());scores[key]+=6*len(matched)
            groups[key].update(matched)
            if len(xrefs)<35:xrefs.append({'from':str(x.getFromAddress()),'function':key,
                                          'kind':str(x.getReferenceType())})
        string_hits.append({'at':str(d.getAddress()),'groups':matched,
                            'text':text_value[:220],'references':xrefs})
    except Exception:continue
for key,fn in functions.items():
    name=str(fn.getName())
    for category,expression in compiled.items():
        if expression.search(name):
            scores[key]+=15
            groups[key].add(category)
    scores[key]+=min(len(callers[key]),20)*2
    scores[key]+=min(len(callees[key]),20)
    scores[key]+=min(len(indirect[key]),10)*8
for v in seeds.get(role,[]):
    try:
        f=fm.getFunctionContaining(program.getAddressFactory().getDefaultAddressSpace().getAddress(v))
        if f:
            key=str(f.getEntryPoint());scores[key]+=200;groups[key].add('known_static_anchor')
    except Exception:pass
# Two hops around the highest priority targets to distinguish dispatcher
# callers and library functions from leaf helpers.
anchors=sorted(functions,key=lambda k:(-scores[k],k))[:min(65,limit)]
for k in anchors:
    for neighbor in callers[k]|callees[k]:
        if neighbor in functions:
            scores[neighbor]+=12
            groups[neighbor].add('neighbor_of_priority_function')
selected=sorted(functions,key=lambda k:(-scores[k],k))[:limit]

interface=DecompInterface()
interface.toggleCCode(True)
interface.setSimplificationStyle('decompile')
if not interface.openProgram(program):raise RuntimeError('Decompiler failed')
entries={}
try:
    for key in selected:
        fn=functions[key]
        record={'name':str(fn.getName()),'address':key,
                'size':int(fn.getBody().getNumAddresses()),
                'score':scores[key],'categories':sorted(groups[key]),
                'direct_callers':sorted(callers[key])[:70],
                'direct_callees':sorted(callees[key])[:100],
                'computed_call_sites':indirect[key][:45]}
        try:
            decoded=interface.decompileFunction(fn,65,monitor)
            if decoded and decoded.decompileCompleted() and decoded.getDecompiledFunction():
                code=str(decoded.getDecompiledFunction().getC())
                code=re.sub(r'"[^"\n]*(?:password|secret|private|token|credential)[^"\n]*"',
                            '"[REDACTED]"',code,flags=re.I)
                record['pseudocode']=code[:17000]
                record['pseudocode_truncated']=len(code)>17000
            else:
                record['decompile_error']=str(decoded.getErrorMessage()) if decoded else 'no decompile result'
        except Exception as exc:
            record['decompile_error']=type(exc).__name__+': '+str(exc)[:220]
        entries[key]=record
finally:interface.dispose()
report={'schema':'mpfree-directed-boot-updater-v8',
        'timestamp_utc':datetime.now(timezone.utc).isoformat(),
        'role':role,'source_sha256_prechecked_by_launcher':digest,
        'program':str(program.getName()),'language':str(program.getLanguageID()),
        'recognized_functions':len(functions),'analyzed_functions':len(entries),
        'selected_function_details':entries,
        'selected_string_cross_references':string_hits,
        'computed_call_sites':{k:v for k,v in indirect.items() if v},
        'warnings':['Static Ghidra evidence is not proof of runtime execution.',
            'The launcher checks the private file SHA; imported ELF memory changes are not the same bytes.',
            'Possible internal credentials and copyrighted pseudocode: PRIVATE report, never commit.',
            'No Akai code was executed and no hardware or storage was modified.',
            'Import table stubs and ARM/Thumb indirect calls can hide command dispatch.',
            'An updater may possess flash-writing code without the 3.10 rootfs-only image invoking it.']}
destination.parent.mkdir(parents=True,exist_ok=True)
mask=os.umask(0o077)
try:destination.write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
finally:os.umask(mask)
print('MPFree V8',role,'functions',len(functions),'details',len(entries),'strings',len(string_hits),'output',destination)