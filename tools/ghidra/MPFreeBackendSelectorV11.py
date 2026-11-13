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
 'spl':[0xff704004,0xff704060,0xff704068,0xff704120,0xff7041a8,
        0xff704160,0xff704164,0xff7041e8,0xff7089fc,0xff708770,
        0xff705af2,0xff705b16,0xff705208,0xff7052c0,
        0xff705310,0xff705316,0xff705342,0xff705350,0xff705418,0xff705480,
        0xff7054a4,0xff7056a8,0xff7069e0,0xff707d28,0xff707de4,
        0xff708b74],
 'uboot':[0x100fe4,0x1011e0,0x100da0,0x101744,0x1017fc,
          0x101bb4,0x10228c,0x1018fc,0x1021a8,0x10a6a4,
          0x10b74c,0x10cc64,0x10d14c,0x10d588,0x10e000,0x10e02a,
          0x10e390,
          0x10f188,0x10f388,0x10f45c,0x10f728,
          0x110084,0x1100c8,0x110270,0x110484,0x112228,0x11622a,
          0x10fde8,0x100edc,0x100ef0,0x1147f0,0x115f30,
          0x115ef4,0x115ed8,0x1147d4,0x1146cc,
          0x1147a2,0x115e98,0x11415e,0x1140ac,0x115d04,
          0x115f90,0x11a9f8,0x11aa10,
          0x12247c,0x1226b4,0x122734,0x122780,
          0x122834,0x122854,0x122980,0x122a10,0x122948,0x122cf8,0x122d38,0x122bd0,0x122bf8,0x122c5c]
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
    lim={'az01-update':30,'libaz01-prog':180,'spl':80,'uboot':160}[role]
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
                'libaz01-prog':('0x0001738c','0x00017554','0x0001798c',
                                 '0x0001f094','0x0001a314','0x0001a674',
                                 '0x0001a8d0','0x0001aabc','0x0001d644',
                                 '0x0001a178','0x0001b35c','0x0001afd4'),
                'spl':('0xff704004','0xff704120','0xff7089fc','0xff708770',
                       '0xff705af2','0xff705b16','0xff7041a8',
                       '0xff7052c0','0xff70529c','0xff705310',
                       '0xff705316','0xff705342','0xff705350','0xff7054a4',
                       '0xff7056a8','0xff708b74'),
                'uboot':('0x00100fe4','0x001011e0','0x00101744','0x001017fc','0x00101bb4','0x0010a6a4','0x0010f388','0x00112228','0x0012247c','0x001226b4','0x00122734','0x00122780','0x00122834','0x00122854','0x00122964','0x00122980','0x00122948','0x00122cf8','0x00122d38','0x00122bf8','0x00122c5c','0x00110084','0x001100c8','0x00110270','0x0010fde8','0x00100fe4','0x00100edc','0x00100ef0','0x001147f0','0x00115f30','0x00115ef4','0x001147d4',
                         '0x001011e0','0x001146cc','0x001147a2',
                         '0x00115e98','0x0011415e','0x001140ac',
                         '0x00115d04','0x00115f90',
                         '0x00122a10','0x0010e390','0x0010e000','0x0010e02a'),
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
    direct_boot={}
    # Other independent research tracks. Original-memory words only,
    # never execute SPL handoff, update trampolines or ARM MMIO operations.
    if role in ('spl','az01-update'):
        slots=(
            {
                'candidate_stage_entry_handoff':0xff705274,
                'boot_device_order_ptr':0xff7053fc,
                'boot_device_mapping_table':0xff705404,
                'stage_handoff_param':0xff70540c,
                'stage_boot_buffer':0xff705598,
                'stage_boot_context':0xff7055a4,
            }
            if role=='spl' else {
                'usb_update_mode_trampoline':0x00012f98,
                'apply_image_trampoline':0x00012fac,
                'download_image_trampoline':0x00012fd0,
            }
        )
        original_pointers={}
        for label,address in slots.items():
            word=None
            try:
                block=mem.getBlock(A(address))
                if block and block.isInitialized():
                    word=int(mem.getInt(A(address))) & 0xffffffff
            except Exception:
                word=None
            fn_match=None
            if word is not None:
                try:
                    target=fm.getFunctionContaining(A(word & 0xfffffffe))
                    if target:
                        fn_match=addr_hex(target.getEntryPoint().getOffset())
                except Exception:
                    pass
            original_pointers[label]={
                'slot':addr_hex(address),
                'original_word':addr_hex(word) if word is not None else None,
                'possible_code_target':fn_match,
                'original_word_proves_execution':False,
                'runtime_target_unverified':True,
            }
        # SPL: resolve previously hidden FIRST indirect jump, then
        # inspect the SECOND indirect pointer without claiming execution.
        # Read bounded media tables as opaque 32-bit words, not guessed
        # eMMC/SD priorities until values and references are checked.
        extended={}
        if role=='spl':
            def original_word(pos):
                try:
                    b=mem.getBlock(A(pos))
                    if b and b.isInitialized():
                        return int(mem.getInt(A(pos))) & 0xffffffff
                except Exception:
                    pass
                return None
            def original_ascii(ptr, max_chars=48):
                try:
                    block=mem.getBlock(A(ptr))
                    if block is None or not block.isInitialized():
                        return None
                    chars=[]
                    for n in range(max_chars):
                        char=int(mem.getByte(A(ptr+n))) & 255
                        if char==0:return ''.join(chars) if chars else None
                        if char<32 or char>126:return None
                        chars.append(chr(char))
                except Exception:
                    pass
                return None
            def original_references_to(addr):
                values=[]
                try:
                    refs=prog.getReferenceManager().getReferencesTo(A(addr))
                    for ref in refs:
                        if len(values)>=48:break
                        values.append({
                            'from':addr_hex(ref.getFromAddress().getOffset()),
                            'type':str(ref.getReferenceType()),
                        })
                except Exception:
                    pass
                return values
            table_words={}
            for name, pointer_slot, count in (
                ('device_order',0xff7053fc,5),
                ('device_mapping',0xff705404,8)):
                address=original_word(pointer_slot)
                words=[]
                if address is not None:
                    for n in range(count):
                        value=original_word(address+n*4)
                        words.append({
                            'address':addr_hex(address+n*4),
                            'word':addr_hex(value) if value is not None else None,
                            'original_ascii_at_pointer':(
                                original_ascii(value) if (
                                    name=='device_mapping' and n%2==1
                                    and value is not None) else None),
                            'direct_static_xrefs':original_references_to(
                                address+n*4)})
                table_words[name]={'origin_slot':addr_hex(pointer_slot),
                                   'table_start':addr_hex(address) if address is not None else None,
                                   'entries':words,
                                   'interpretation_not_verified':True}
            context=original_word(0xff70540c)
            legacy_header_original_words={
                'image_header_magic_literal_word':(
                    addr_hex(original_word(0xff705308))
                    if original_word(0xff705308) is not None else None),
                'image_header_context_ptr':(
                    addr_hex(original_word(0xff70530c))
                    if original_word(0xff70530c) is not None else None),
                'image_header_default_context_ptr':(
                    addr_hex(original_word(0xff7052b8))
                    if original_word(0xff7052b8) is not None else None),
                'default_header_data_ptr':(
                    addr_hex(original_word(0xff7052bc))
                    if original_word(0xff7052bc) is not None else None),
                'context_header_flag_at_plus_0x14':(
                    addr_hex(original_word(context+0x14))
                    if context is not None and
                    original_word(context+0x14) is not None else None),
            }
            handoff_slot=context+0xc if context is not None else None
            handoff_value=original_word(handoff_slot) if handoff_slot is not None else None
            entry=original_word(0xff705274)
            extended={
                'first_indirect_call_slot':'0xff705274',
                'first_indirect_call_target':addr_hex(entry & 0xfffffffe) if entry is not None else None,
                'first_indirect_thumb_bit':bool(entry & 1) if entry is not None else None,
                'first_indirect_call_matches_boot_selector':entry==0xff705351,
                'boot_selector_decomp_reaches_indirect_context':
                    'FUN_ff705310(DAT_ff70540c)' in details.get(
                        '0xff705350',{}).get('pseudocode',''),
                'indirect_handoff_context_address':addr_hex(context) if context is not None else None,
                'indirect_handoff_function_pointer_slot':addr_hex(handoff_slot) if handoff_slot is not None else None,
                'indirect_handoff_original_word':addr_hex(handoff_value) if handoff_value is not None else None,
                'indirect_handoff_original_word_is_runtime_proof':False,
                'second_handoff_initial_word_zero':handoff_value==0,
                'second_handoff_pointer_slot_static_xrefs':(
                    original_references_to(handoff_slot) if
                    handoff_slot is not None else []),
                'legacy_image_header_original_words':legacy_header_original_words,
                'boot_order_initialization_writer_decompiled':(
                    'FUN_ff704120()' in details.get(
                        '0xff705342',{}).get('pseudocode','')
                    and '*param_1 = uVar1;' in details.get(
                        '0xff705342',{}).get('pseudocode','')),
                'opaque_media_table_words':table_words,
            }
        elif role=='az01-update':
            # GOT/PLT veneers are LINKAGE references. A mapped
            # stub address isn't necessarily the implementation.
            extended={'trampoline_candidates':{
                label:{
                    'imported_plt_or_got_candidate':True,
                    'actual_shared_object_resolved':False,
                    'original_linkage_slot_address':item['slot'],
                    'candidate_code_address':item['original_word'],
                }
                for label,item in original_pointers.items()
            }}
        direct_boot={
            'schema':'mpfree-original-stage-pointer-audit-v1',
            'role':role,
            'original_image_pointers':original_pointers,
            'extended_stage_evidence':extended,
            'does_not_run_vendor_code':True,
            'indirect_spl_handoff_target_proven':False,
            'live_updater_trampoline_target_proven':False,
        }
    # Owner-private library image-format dispatch probes. The
    # az01_open_image routine selects format handlers through PIC/GOT
    # expressions, so raw words here are NOT resolved runtime methods.
    if role=='libaz01-prog':
        slots=(
            ('az01_open_image_pic_base',0x000174f4),
            ('az01_open_image_AZ01_offset',0x000174f8),
            ('az01_open_image_AZ0x_offset',0x000174fc),
            ('az01_open_image_FDT_offset',0x00017500),
        )
        saved={}
        for label,address in slots:
            raw=None
            block=mem.getBlock(A(address))
            if block is not None and block.isInitialized():
                try:
                    raw=int(mem.getInt(A(address))) & 0xffffffff
                except Exception:
                    raw=None
            saved[label]={
                'address':addr_hex(address),
                'original_memory_word':addr_hex(raw) if raw is not None else None,
                'runtime_relocation_or_callback_not_proven':True,
            }
        # Evaluate the PIC-relative pointer-table slots described
        # by az01_open_image. This reads original Ghidra memory only.
        # The pointer table may already be relocated by import, and
        # each slot points to a handler descriptor, NOT necessarily
        # directly to executable code.
        raw_base=saved['az01_open_image_pic_base']['original_memory_word']
        format_table_slot_words={}
        for label in ('AZ01','AZ0x','FDT'):
            offset_key='az01_open_image_'+label+'_offset'
            offset_word=saved[offset_key]['original_memory_word']
            computed=None
            if raw_base is not None and offset_word is not None:
                computed=(int(raw_base,16)+0x173c0+int(offset_word,16))
            value=None
            entry_first_word=None
            if computed is not None:
                try:
                    b=mem.getBlock(A(computed))
                    if b is not None and b.isInitialized():
                        value=int(mem.getInt(A(computed))) & 0xffffffff
                except Exception:
                    pass
                if value is not None:
                    try:
                        b=mem.getBlock(A(value))
                        if b is not None and b.isInitialized():
                            entry_first_word=int(mem.getInt(A(value))) & 0xffffffff
                    except Exception:
                        pass
            format_table_slot_words[label]={
                'computed_PIC_table_slot':addr_hex(computed) if computed is not None else None,
                'descriptor_pointer_image_word':addr_hex(value) if value is not None else None,
                'descriptor_first_word_image':addr_hex(entry_first_word) if entry_first_word is not None else None,
                'dynamic_PIC_relocation_and_executable_target_unproven':True,
            }
        direct_boot={
            'schema':'mpfree-original-image-format-dispatch-words-v1',
            'source_stage':'libaz01-prog',
            'image_format_prefixes':['AZ0x','AZ01','D00DFEED'],
            'format_handler_candidate_original_words':saved,
            'computed_PIC_descriptor_table_slots':format_table_slot_words,
            'no_vendor_execution':True,
            'runtime_dispatch_resolved':False,
        }
    # Explicit P3 direct-boot/protocol research. Read literal-pool values and
    # original bytes from the SHA-verified saved project. No code execution.
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
            'pmu_syscon_register_base':0xff730000,
            'pmu_reboot_mode_register':0xff730094,
            'bootcount_upper16_magic':0xb0010000,
            'bootcount_halfword_magic':0x0000b001,
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
        # Resolve the eight-entry Fastboot prefix/callback dispatch table
        # used at 0x122780. This independently distinguishes download,
        # boot, flash and OEM entry gates. Never call a firmware handler.
        command_table_pointer=read_word(0x00122818)
        command_dispatch=[]
        if command_table_pointer is not None:
            for index in range(8):
                pair=command_table_pointer+index*8
                prefix_ptr=read_word(pair)
                callback_ptr=read_word(pair+4)
                prefix=read_cstr(prefix_ptr,48) if prefix_ptr is not None else None
                callback=(callback_ptr & 0xfffffffe) if callback_ptr is not None else None
                callback_fn=None
                if callback is not None:
                    try:
                        candidate=fm.getFunctionContaining(A(callback))
                        if candidate:
                            callback_fn=addr_hex(candidate.getEntryPoint().getOffset())
                    except Exception:
                        pass
                command_dispatch.append({
                    'slot_index':index,
                    'table_pair_address':addr_hex(pair),
                    'prefix_pointer':addr_hex(prefix_ptr) if prefix_ptr is not None else None,
                    'prefix_text':prefix,
                    'raw_callback_pointer':addr_hex(callback_ptr) if callback_ptr is not None else None,
                    'callback_address':addr_hex(callback) if callback is not None else None,
                    'callback_containing_function':callback_fn,
                    'callback_is_thumb':bool(callback_ptr & 1) if callback_ptr is not None else None,
                })
        # Look at original ELF mapping, NOT runtime RAM contents.
        # An uninitialized/BSS byte is not an inferred live unlock state.
        def original_word_metadata(pos):
            try:
                block=mem.getBlock(A(pos))
                if block is None:return {'mapped':False}
                initialized=bool(block.isInitialized())
                data={
                    'mapped':True,
                    'memory_block':str(block.getName()),
                    'initialized_in_original_image':initialized,
                    'writeable_at_runtime':bool(block.isWrite()),
                    'original_image_word':(
                        addr_hex(read_word(pos)) if initialized
                        else None),
                    'runtime_word_unknown':True,
                }
                return data
            except Exception as exc:
                return {'mapped':None,'error':str(exc),
                        'runtime_word_unknown':True}
        reboot_mode_literal_map={}
        for label,slot in (
            ('mode_register_pointer',0x0010114c),
            ('mode_prefix_constant',0x00101150),
            ('recovery_flag_pointer',0x00101154),
            ('reboot_log_string_pointer',0x00101164),
            ('recovery_adc_not_found_string',0x00101158),
            ('recovery_adc_start_failed_string',0x00101168),
            ('recovery_adc_read_failed_string',0x0010116c),
            ('board_setup_recovery_flag_pointer',0x001013ac),
            ('board_setup_recovery_env_value_string',0x001013b0),
            ('board_setup_dorecovery_env_name_string',0x001013b4),
            ('getvar_adc_key_string',0x00100fb0),
            ('getvar_adc_operation0_failure_string',0x00100fb8),
            ('getvar_adc_operation1_failure_string',0x00100fbc),
            ('getvar_power_source_reference_error_string',0x00100fc0),
            ('getvar_power_source_scale',0x00100fc4),
            ('getvar_power_source_threshold_1',0x00100fc8),
            ('getvar_power_source_label_1_string',0x00100fcc),
            ('getvar_power_source_threshold_2',0x00100fd0),
            ('getvar_power_source_label_2_string',0x00100fd4),
            ('getvar_power_source_threshold_3',0x00100fd8),
            ('getvar_power_source_label_3_string',0x00100fdc),
            ('getvar_power_source_label_4_string',0x00100fe0),
        ):
            value=read_word(slot)
            reboot_mode_literal_map[label]={
                'slot':addr_hex(slot),
                'value':addr_hex(value) if value is not None else None,
                'deref_original_word':(
                    original_word_metadata(value) if value is not None
                    else None),
                'printable_string':(
                    read_cstr(value,128) if value is not None and
                    ('string' in label or 'env_name' in label)
                    else None),
            }
        original_state={
            'fastboot_permission_global_0x14d690':
                original_word_metadata(0x0014d690),
            'boot_download_word_at_0x143e34':
                original_word_metadata(0x00143e34),
        }
        # The callbacks below are the ordinary Fastboot registered
        # completion functions; their presence does not establish that
        # download/boot is authorized in a locked session.
        completion_callbacks={
            'boot_registration_literal':original_word_metadata(0x00122978),
            'boot_registration_target':read_word(0x00122978),
            'continue_registration_literal':original_word_metadata(0x0012295c),
            'continue_registration_target':read_word(0x0012295c),
        }
        # Both the request-length handler and the receive callback
        # reference globals through original little-endian literal
        # slots. Equality of these pointers is a static hypothesis
        # until verified; values are NOT observed runtime RAM state.
        download_ram_slots=(
            ('download_requested_length_global',0x001229f4),
            ('download_received_cursor_global',0x001229f8),
            ('receive_expected_length_global',0x00122ad4),
            ('receive_received_cursor_global',0x00122ac4),
            ('receive_buffer_early_pointer',0x001224b0),
            ('receive_buffer_late_pointer',0x001224b4),
            ('bootm_argument_string',0x001226f4),
            ('bootm_diagnostic_string',0x001226f8),
            ('bootm_argument_value_pointer',0x001226fc),
            ('bootm_argument_format',0x00122700),
        )
        download_ram_words={
            label:{
                'original_literal_slot':addr_hex(address),
                'original_literal_word':addr_hex(read_word(address))
                    if read_word(address) is not None else None,
                'original_image_word_not_runtime_state':True,
            }
            for label,address in download_ram_slots
        }
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
            'original_image_word_metadata_not_runtime':original_state,
            'reboot_mode_original_literal_slots':reboot_mode_literal_map,
            'recovery_adc_to_environment_pointer_equal':(
                reboot_mode_literal_map[
                    'recovery_flag_pointer']['value'] is not None
                and reboot_mode_literal_map[
                    'recovery_flag_pointer']['value'] ==
                    reboot_mode_literal_map[
                    'board_setup_recovery_flag_pointer']['value']),
            'recovery_button_adc_strings':[
                reboot_mode_literal_map[label]
                for label in (
                    'recovery_adc_not_found_string',
                    'recovery_adc_start_failed_string',
                    'recovery_adc_read_failed_string')
            ],
            'recovery_threshold_predicate':(
                'if (local_1c[0] < 100)' in
                details.get('0x00100fe4',{}).get('pseudocode','')),
            'recovery_threshold_source_function_candidates':[
                '0x001147f0','0x00115ef4','0x00115f30',
                '0x00115ed8','0x001147d4'
            ],
            'registered_callback_pointer_metadata':completion_callbacks,
            'fastboot_download_ram_original_literal_slots':download_ram_words,
            'request_and_receive_length_global_pointer_equal':(
                download_ram_words['download_requested_length_global'][
                    'original_literal_word'] is not None and
                download_ram_words['download_requested_length_global'][
                    'original_literal_word'] ==
                download_ram_words['receive_expected_length_global'][
                    'original_literal_word']),
            'request_and_receive_cursor_pointer_equal':(
                download_ram_words['download_received_cursor_global'][
                    'original_literal_word'] is not None and
                download_ram_words['download_received_cursor_global'][
                    'original_literal_word'] ==
                download_ram_words['receive_received_cursor_global'][
                    'original_literal_word']),
            'fastboot_command_table_pointer':(
                addr_hex(command_table_pointer) if command_table_pointer is not None else None),
            'fastboot_command_table':command_dispatch,
            'fastboot_command_table_expected_eight_entries':len(command_dispatch)==8,
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
