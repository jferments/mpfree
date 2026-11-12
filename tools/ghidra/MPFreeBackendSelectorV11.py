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
 'az01-update':[0x1091c,0x10be4],
 'libaz01-prog':[0x14154,0x145d8,0x149b8,0x14ac8,0x14b98,0x14c50,
 0x14d00,0x14dc4,0x154b8,0x157a8,0x15bec,0x16028,0x1611c,0x16210,
 0x163ec,0x166f8,0x16c14,0x16dbc,0x1738c,0x17554,0x1798c,
 0x18ccc,0x1c208,0x1c3fc,0x1c49c,0x1c6b8,0x1c73c,0x1c8c8,
 0x1e194,0x1f094,0x1b9bc,0x1bf2c],
 'spl':[0xff704004,0xff7041a8,0xff7052c0,0xff705310,
        0xff705350,0xff7054a4,0xff7056a8],
 'uboot':[0x100fe4,0x1011e0,0x101744,0x1017fc,0x101bb4,
          0x10228c,0x10a6a4,0x112228,0x122834,0x122854,0x122980,
          0x1226b4,0x122c5c,0x122bf8]
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
    if backend_file!='none':
        doc=json.loads(Path(backend_file).read_text())
        if doc.get('source_sha256')!=expect or doc.get('role')!=role:
            raise RuntimeError('Backend table file source/role mismatch')
        for t in doc.get('tables',[])[:35]:
            table_va=int(t['base_vaddr'],16)
            table_abs=base+table_va
            xrefs=[];slots=[]
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
                           'observed_ghidra_slots':slots,
                           'table_assignment_proven':False})
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
    lim={'az01-update':22,'libaz01-prog':180,'spl':25,'uboot':55}[role]
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
    out={'schema':'mpfree-v11-backend-selector-and-boot-recovery-static','role':role,
         'source_sha256_prechecked_by_launcher':expect,
         'boot_stage_memory_sha_verified':role in ('spl','uboot'),
         'ghidra_image_base':addr_hex(base),
         'verified_raw_table_candidates':tables,
         'analyzed_functions':details,
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
