# @runtime PyGhidra
# @category MPFree
# V11 focused backend/verification/ROM static decompilation.
# Source SHA verified by offline runner; P1/P3 Ghidra bytes verified here.
# NO vendor execution and no access to MPC/USB/eMMC/OTP.
import hashlib
import json
import os
import re
from pathlib import Path
from ghidra.app.decompiler import DecompInterface
from jpype import JArray,JByte

SHA={
 'az01-update':'57511e1d503f3ceaae3cb9bc40174e073b7ae410e65c986f1991731c77ae655b',
 'libaz01-prog':'af613a88f1b089069ed81a5b5fdf1ceb8c8f68bcd43c7d26b31c9ec9590e4623',
 'spl':'6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f',
 'uboot':'b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a'
}
PINNED={
 'az01-update':[0x1091c,0x10be4,0x10d60,0x11248,0x10850,0x1088c,0x108e0,0x108f8],
 'libaz01-prog':[0x14154,0x145d8,0x149b8,0x14ac8,0x14b98,0x14c50,
 0x14d00,0x14dc4,0x154b8,0x157a8,0x15bec,0x16028,0x1611c,0x16210,
 0x163ec,0x166f8,0x16c14,0x16dbc,0x1738c,0x17554,0x1798c,
 0x18ccc,0x1c208,0x1c3fc,0x1c49c,0x1c6b8,0x1c73c,0x1c8c8,
 0x1e194,0x1f094,0x1b9bc,0x1bf2c,
 0x1d924,0x1d9fc,0x1d274,0x1d374,0x1d644,
 0x16ff8,0x1b49c,0x1ca80,
 0x14fc4,0x152b0,0x153ac,0x15714,0x158fc,
 0x164b0,0x165f0,0x1a178,0x1a314,0x1a674,0x1a8d0,
 0x1a9bc,0x1aabc,0x1afd4,0x1b35c,0x1b27c,0x1b150,
 0x1be1c,0x1c2bc,0x1d8c4],
 'spl':[0xff704004,0xff704060,0xff704068,0xff7041a8,
        0xff704160,0xff704164,0xff7041e8,0xff705208,0xff7052c0,
        0xff705310,0xff705316,0xff705350,0xff705418,0xff705480,
        0xff7054a4,0xff7056a8,0xff7069e0,0xff707d28,0xff707de4,
        0xff708b74],
 'uboot':[0x100fe4,0x1011e0,0x100da0,0x101744,0x1017fc,
          0x101bb4,0x10228c,0x1018fc,0x1021a8,0x10a6a4,
          0x10b74c,0x10cc64,0x10d14c,0x10d588,0x10e000,0x10e02a,
          0x10f188,0x10f388,0x10f45c,0x10f728,
          0x1100c8,0x110270,0x110484,0x112228,0x11622a,
          0x12247c,0x1226b4,0x122734,0x122780,
          0x122834,0x122854,0x122980,0x122bd0,0x122bf8,0x122c5c]
}
OFFSETS={'open':0x08,'write_bootloader':0x18,'write_partition':0x1c,
         'stream_partition':0x20,'commit_bootloader':0x2c,'reset':0x34}

def mpfree_v11_entrypoint():
    args=list(getScriptArgs())
    if len(args)!=4:raise RuntimeError('role report-path verified-sha backend-report-or-none')
    role,dest,expect,backend_file=str(args[0]),Path(str(args[1])),str(args[2]),str(args[3])
    if role not in SHA or expect!=SHA[role] or not dest.is_absolute():
        raise RuntimeError('V11 role/path/source hash mismatch')
    prog=currentProgram
    if prog is None or 'ARM:' not in str(prog.getLanguageID()):
        raise RuntimeError('Wrong Ghidra project architecture')
    mem=prog.getMemory();fm=prog.getFunctionManager()
    rm=prog.getReferenceManager();listing=prog.getListing()
    addrspace=prog.getAddressFactory().getDefaultAddressSpace()
    base=int(prog.getImageBase().getOffset())
    def A(i):return addrspace.getAddress(int(i))
    def addr_hex(i):return '0x%08x'%int(i)
    if role in ('spl','uboot'):
        start,length=((0xff704000,32768) if role=='spl' else (0x100000,363649))
        block=mem.getBlock(A(start))
        if block is None or block.getStart().getOffset()!=start or block.getSize()!=length:
            raise RuntimeError('Unexpected saved P1/P3 in-memory block')
        h=hashlib.sha256();buf=JArray(JByte)(32768)
        for off in range(0,length,32768):
            n=min(32768,length-off)
            if int(mem.getBytes(A(start+off),buf,0,n))!=n:
                raise RuntimeError('Cannot verify owner SPL/U-Boot bytes')
            h.update(bytes(int(b)&255 for b in buf[:n]))
        if h.hexdigest()!=expect:
            raise RuntimeError('Saved project differs from verified owner boot stage')
    selected=set(PINNED[role])
    # Fresh Ghidra ELF imports do not necessarily recognize stripped
    # entrypoints (notably az01-update main at 0x1091c). Recover pinned
    # executable entry functions in the disposable imported project only.
    if role in ('az01-update','libaz01-prog'):
        for va in sorted(selected):
            a=A(va)
            block=mem.getBlock(a)
            if not block or not block.isExecute() or fm.getFunctionContaining(a):
                continue
            try:
                disassemble(a)
                createFunction(a,'mpfree_v11_pinned_%x'%va)
            except Exception:
                pass
    tables=[]
    table_xref_targets=set()
    if backend_file!='none':
        doc=json.loads(Path(backend_file).read_text())
        if doc.get('source_sha256')!=expect or doc.get('role')!=role:
            raise RuntimeError('Backend table file source/role mismatch')
        for t in doc.get('tables',[])[:35]:
            table_va=int(t['base_vaddr'],16)
            table_abs=base+table_va
            xrefs=[];slots=[];source_functions=set();xref_sites=[]
            for delta in (0,4,8,0x18,0x1c,0x20,0x2c,0x34):
                pos=table_abs+delta
                b=mem.getBlock(A(pos))
                if not b or not b.isInitialized():continue
                word=int(mem.getInt(A(pos)))&0xffffffff
                # Imported ARM PIE has already been relocated by Ghidra.
                # Try the absolute word first. Adding base again was a V10 bug.
                raw_target=word&~1
                target_fn=None
                for cand in (raw_target,base+raw_target):
                    if cand==0:continue
                    try:
                        target_fn=fm.getFunctionContaining(A(cand))
                    except Exception:pass
                    if target_fn:break
                match=target_fn.getEntryPoint().getOffset() if target_fn else None
                if match is not None:
                    selected.add(match)
                refs=[]
                for ref in rm.getReferencesTo(A(pos)):
                    refs.append(str(ref.getFromAddress()))
                    source_fn=fm.getFunctionContaining(ref.getFromAddress())
                    if source_fn:
                        entry=source_fn.getEntryPoint().getOffset()
                        source_functions.add(entry)
                        table_xref_targets.add(entry)
                    if len(xref_sites)<80:
                        ins=listing.getInstructionContaining(ref.getFromAddress())
                        xref_sites.append({
                            'slot_relative':delta,
                            'source':str(ref.getFromAddress()),
                            'source_function':str(source_fn.getEntryPoint()) if source_fn else None,
                            'instruction':str(ins) if ins else None,
                            'previous_instruction':str(ins.getPrevious()) if ins and ins.getPrevious() else None,
                            'next_instruction':str(ins.getNext()) if ins and ins.getNext() else None,
                        })
                    if len(refs)>=25:break
                xrefs+=refs
                slots.append({'relative':delta,'absolute':addr_hex(pos),
                              'word':addr_hex(word),'resolved_function':addr_hex(match) if match is not None else None,
                              'reference_from':refs})
            tables.append({'base_elf_vaddr':t['base_vaddr'],
                           'imported_base_address':addr_hex(table_abs),
                           'raw_header':t['header_words'],
                           'raw_callbacks':t['callbacks'],
                           'code_xrefs_to_table_or_slots':xrefs[:60],
                           'xref_source_functions':[addr_hex(i) for i in sorted(source_functions)],
                           'xref_site_instructions':xref_sites,
                           'observed_ghidra_slots':slots,
                           'table_assignment_proven':False})
    # Investigate the containing functions of actual references to candidate
    # tables; these are where backend selection/assignment may occur. Never
    # invent a new function starting at an arbitrary interior xref address.
    selected.update(table_xref_targets)
    # Recover every relocation-backed callback function before decompiling.
    # May change only temporary imported ELF programs, never P1/P3.
    if role in ('az01-update','libaz01-prog'):
        for t in tables:
            for slot in t['observed_ghidra_slots']:
                if slot['relative'] not in OFFSETS.values():continue
                word=int(slot['word'],16)&~1
                if not word:continue
                addr=A(word)
                block=mem.getBlock(addr)
                if not block or not block.isExecute():continue
                fn=fm.getFunctionContaining(addr)
                if fn is None:
                    try:
                        disassemble(addr)
                        fn=createFunction(addr,'mpfree_v11_callback_%x'%word)
                    except Exception:continue
                if fn is not None:
                    slot['resolved_function']=str(fn.getEntryPoint())
                    selected.add(fn.getEntryPoint().getOffset())
    # Add one-hop caller/callee references; prioritize table callbacks and
    # verifier/write entrypoints. Do not assert indirect callee edges.
    edges={}
    original=set(selected)
    for va in sorted(original):
        fn=fm.getFunctionContaining(A(va))
        if not fn:continue
        key=fn.getEntryPoint().getOffset()
        callers=set();callees=set();computed=[]
        for ref in rm.getReferencesTo(fn.getEntryPoint()):
            caller=fm.getFunctionContaining(ref.getFromAddress())
            if caller:callers.add(caller.getEntryPoint().getOffset())
        it=listing.getInstructions(fn.getBody(),True)
        while it.hasNext():
            ins=it.next();ft=ins.getFlowType()
            if ft.isCall():
                for target in ins.getFlows():
                    callee=fm.getFunctionContaining(target)
                    if callee:callees.add(callee.getEntryPoint().getOffset())
            if ft.isComputed() and len(computed)<60:
                computed.append({'at':str(ins.getAddress()),'instruction':str(ins)})
        edges[key]={'callers':[addr_hex(x) for x in sorted(callers)[:70]],
                    'callees':[addr_hex(x) for x in sorted(callees)[:70]],
                    'computed':computed}
        for near in list(callers)[:8]+list(callees)[:8]:
            if fm.getFunctionContaining(A(near)):selected.add(near)
    priority=sorted(selected,key=lambda va:(va not in original, va))
    # Expanded direction: ROM entry, loader handoff, fastboot auth and update flow.
    lim={'az01-update':30,'libaz01-prog':180,'spl':45,'uboot':100}[role]
    if not priority:raise RuntimeError('No functions selected for analysis')
    decoder=DecompInterface();decoder.toggleCCode(True)
    if not decoder.openProgram(prog):raise RuntimeError('Decompiler unavailable')
    details={}
    try:
        for va in priority[:lim]:
            fn=fm.getFunctionContaining(A(va))
            if not fn:continue
            key=addr_hex(fn.getEntryPoint().getOffset())
            record={'name':str(fn.getName()),
                    'size':int(fn.getBody().getNumAddresses()),
                    'graph':edges.get(fn.getEntryPoint().getOffset(),{}),
                    'pinned':fn.getEntryPoint().getOffset() in original}
            # Raw ARM instruction record for the small set of functions
            # where Ghidra C return types and structure offsets are uncertain.
            # These records are written ONLY to the private owner ZIP.
            capture_by_role={
                'libaz01-prog':('0x0001a314','0x0001a674','0x0001a8d0','0x0001aabc','0x0001d644','0x0001a178','0x0001b35c','0x0001afd4'),
                'spl':('0xff704004','0xff7041a8','0xff705310','0xff705316','0xff705350','0xff7054a4','0xff7056a8','0xff708b74'),
                'uboot':('0x00100fe4','0x001011e0','0x00101744','0x001017fc','0x00101bb4','0x0010a6a4','0x0010f388','0x00112228','0x0012247c','0x001226b4','0x00122734','0x00122834','0x00122854','0x00122980','0x00122bf8','0x00122c5c'),
                'az01-update':('0x0001091c','0x00010be4')
            }
            if key in capture_by_role.get(role,()):
                raw_ins=[]
                iterator=listing.getInstructions(fn.getBody(),True)
                while iterator.hasNext() and len(raw_ins)<550:
                    ins=iterator.next()
                    raw_ins.append({'address':str(ins.getAddress()),
                                    'mnemonic':str(ins.getMnemonicString()),
                                    'text':str(ins),
                                    'flow':str(ins.getFlowType())})
                record['machine_instruction_evidence']=raw_ins
                record['instruction_capture_limited']=iterator.hasNext()
            try:
                dec=decoder.decompileFunction(fn,65,monitor)
                if dec and dec.decompileCompleted() and dec.getDecompiledFunction():
                    raw=str(dec.getDecompiledFunction().getC())
                    raw=re.sub(r'"[^"\n]{150,}"','"[LONG_LITERAL_REDACTED]"',raw)
                    raw=re.sub(r'"[^"\n]*(?:password|secret|private.key|credential|token)[^"\n]*"',
                               '"[SENSITIVE_REDACTED]"',raw,flags=re.I)
                    record['pseudocode']=raw[:15000]
                    record['truncated']=len(raw)>15000
                else:
                    record['error']=str(dec.getErrorMessage()) if dec else 'Decompiler returned None'
            except Exception as e:record['error']=type(e).__name__+': '+str(e)[:180]
            details[key]=record
    finally:decoder.dispose()
    # Explicit P3 direct-boot/protocol research. Read literal-pool values and
    # original bytes from the SHA-verified saved project. No code execution.
    direct_boot={}
    if role=='uboot':
        def read_word(pos):
            try:
                block=mem.getBlock(A(pos))
                if not block or not block.isInitialized():return None
                return int(mem.getInt(A(pos))) & 0xffffffff
            except Exception:
                return None
        def read_cstr(pos,limit=96):
            try:
                block=mem.getBlock(A(pos))
                if not block or not block.isInitialized():return None
                outchars=[]
                for i in range(limit):
                    byte=int(mem.getByte(A(pos+i))) & 255
                    if byte==0:break
                    if byte<32 or byte>126:return None
                    outchars.append(chr(byte))
                return ''.join(outchars)
            except Exception:
                return None
        locations={
            'permission_read_literal':0x0012284c,
            'permission_write_literal':0x00122928,
            'oem_unlock_string_literal':0x00122924,
            'oem_prefix_string_literal':0x00122918,
            'download_destination_global_literal':0x001226fc,
        }
        ptrs={}
        for name,address in locations.items():
            value=read_word(address)
            entry={'literal_pool_address':addr_hex(address),
                   'loaded_pointer':addr_hex(value) if value is not None else None}
            if value is not None and 'string' in name:
                decoded=read_cstr(value)
                entry['ascii_length']=len(decoded) if decoded is not None else None
                entry['ascii_sha256']=hashlib.sha256(decoded.encode('ascii')).hexdigest() if decoded is not None else None
                entry['matches_published_37_byte_unlock_token']=bool(
                    decoded is not None and len(decoded)==37 and
                    entry['ascii_sha256']=='f10b2ec4fa713c1f1d1f584934f501a150119acede06e6012006aa1352e8121c')
            ptrs[name]=entry
        rptr=ptrs['permission_read_literal'].get('loaded_pointer')
        wptr=ptrs['permission_write_literal'].get('loaded_pointer')
        global_refs=[]
        for address in (0x0014d690,):
            try:
                for ref in rm.getReferencesTo(A(address)):
                    inst=listing.getInstructionContaining(ref.getFromAddress())
                    fn=fm.getFunctionContaining(ref.getFromAddress())
                    if len(global_refs)>=100:break
                    global_refs.append({
                        'from':str(ref.getFromAddress()),
                        'type':str(ref.getReferenceType()),
                        'function':str(fn.getEntryPoint()) if fn else None,
                        'instruction':str(inst) if inst else None,
                    })
            except Exception:
                pass
        # Look for raw register/magic occurrences in original P3 bytes.
        # Negative findings mean only 'no contiguous 32-bit literal found'.
        import struct
        raw=bytearray()
        off=0
        while off<363649:
            take=min(32768,363649-off)
            chunk=JArray(JByte)(take)
            if int(mem.getBytes(A(0x100000+off),chunk,0,take))!=take:
                raise RuntimeError('Cannot read P3 for direct-boot constant scan')
            raw.extend(int(x)&255 for x in chunk)
            off+=take
        constants={
            'pmu_boot_counter_register':0xff7300a0,
            'pmu_reboot_mode_register':0xff730094,
            'bootcount_upper16_magic':0xb0010000,
            'rom_download_magic':0xef08a53c,
            'fastboot_permission_global':0x0014d690,
        }
        hits={}
        for label,value in constants.items():
            where=[]
            for endian,packed in (('little',struct.pack('<I',value)),
                                  ('big',struct.pack('>I',value))):
                offset=0
                while len(where)<32:
                    pos=raw.find(packed,offset)
                    if pos<0:break
                    where.append({'address':addr_hex(0x100000+pos),
                                  'byte_order':endian})
                    offset=pos+1
            hits[label]=where
        direct_boot={
            'schema':'mpfree-p3-directboot-literal-evidence-v1',
            'permission_read_slot':ptrs['permission_read_literal'],
            'permission_write_slot':ptrs['permission_write_literal'],
            'oem_token_slot':ptrs['oem_unlock_string_literal'],
            'oem_prefix_slot':ptrs['oem_prefix_string_literal'],
            'other_literal_slots':ptrs,
            'permission_read_write_pointer_equal':bool(rptr is not None and rptr==wptr),
            'permission_read_write_pointer_is_prior_global':bool(
                rptr=='0x0014d690' and wptr=='0x0014d690'),
            'permission_global_xrefs':global_refs,
            'raw_word_occurrences':hits,
            'notes':[
                'Literal slot dereference verified from original U-Boot bytes.',
                'Token comparison proves code/data relationship only, no authorization command sent.',
                'No PMU literal does not rule out indirect/computed address construction.',
                'Read-only, no firmware code executed and no device accessed.',
            ],
        }
    out={'schema':'mpfree-v11-backend-selector-and-boot-recovery-static','role':role,
         'source_sha256_prechecked_by_launcher':expect,
         'boot_stage_memory_sha_verified':role in ('spl','uboot'),
         'ghidra_image_base':addr_hex(base),
         'verified_raw_table_candidates':tables,
         'analyzed_functions':details,
         'direct_boot_probe':direct_boot,
         'decompiled':sum('pseudocode' in x for x in details.values()),
         'restrictions':[
           'ELF table labels are hypotheses until assigned by proven runtime selector.',
           'Ghidra pointer words may already include load image base.',
           'Callback offset match alone cannot establish signature enforcement.',
           'The emulator cannot prove RK3288 silicon MaskROM or eMMC physical recovery.',
           'No Akai executable run, no MPC USB/flash/OTP accessed.']}
    dest.parent.mkdir(parents=True,exist_ok=True)
    old=os.umask(0o077)
    try:dest.write_text(json.dumps(out,indent=2)+'\n')
    finally:os.umask(old)
    print('V11',role,'tables',len(tables),'functions',len(details),'decompiled',out['decompiled'])
mpfree_v11_entrypoint()
