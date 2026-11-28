# @runtime PyGhidra
# @category MPFree
# V10 targeted offline static xrefs/relocated tables and function control flow.
# The only modifications are to disposable in-memory Ghidra ELF projects.
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
ADDRESS={
 'az01-update':[0x1091c,0x10be4,0x10d60,0x11248],
 'libaz01-prog':[
   0x13828,0x139ec,0x14154,0x145d8,0x149b8,0x14ac8,0x14b98,
   0x14c50,0x14d00,0x14dc4,0x14ef0,0x14f0c,0x14fc4,
   0x152b0,0x153ac,0x154b8,0x15714,0x158fc,0x159c0,
   0x15bec,0x15da8,0x16028,0x164b0,0x165f0,0x166f8,
   0x16c14,0x16dbc,0x1738c,0x17554,0x1798c,
   0x18ccc,0x19340,0x19538,0x1a314,0x1b9bc,0x1bf2c,
   0x1c208,0x1c2bc,0x1c3fc,0x1c49c,0x1c6b8,0x1c8c8,
   0x1cb74,0x1d0d0,0x1d374,0x1d538,0x1d644,
   0x1d8c4,0x1e194,0x1f094],
 'spl':[0xff704004,0xff7052c0,0xff705310,0xff705350,0xff7054a4],
 'uboot':[0x00100fe4,0x001011e0,0x00101744,0x001017fc,
          0x00101bb4,0x0010fde8,0x00112228,0x00122bf8,
          0x00122c5c,0x001226b4,0x00122834,0x00122980]
}
WORD_PAT=re.compile(r'rockusb|bootrom|maskrom|reboot|bootloader|kexec|USB|verify|signed|'
                    r'partition|write|fastboot|flash|loader|oem|image|usb|mmc|sdcard',re.I)

def mpfree_v10_run():
    args=list(getScriptArgs())
    if len(args)!=4:raise RuntimeError('role absolute-report expected-sha candidate-json-or-none')
    role,dest,expected,candidate=str(args[0]),Path(str(args[1])),str(args[2]),str(args[3])
    if role not in SHA or expected!=SHA[role] or not dest.is_absolute():
        raise RuntimeError('Invalid V10 offline source identity')
    program=currentProgram
    if program is None or 'ARM:' not in str(program.getLanguageID()):
        raise RuntimeError('Expected ARM Ghidra project')
    image_base=int(program.getImageBase().getOffset())
    memory=program.getMemory();space=program.getAddressFactory().getDefaultAddressSpace()
    def A(x):return space.getAddress(int(x))
    fm=program.getFunctionManager();rm=program.getReferenceManager()
    listing=program.getListing()
    if role in ('spl','uboot'):
        start,size=((0xff704000,32768) if role=='spl' else (0x100000,363649))
        b=memory.getBlock(A(start))
        if b is None or int(b.getStart().getOffset())!=start or int(b.getSize())!=size:
            raise RuntimeError('Boot stage memory block different from verified owner backup')
        digest=hashlib.sha256()
        chunk=JArray(JByte)(32768)
        for off in range(0,size,32768):
            n=min(32768,size-off)
            if int(memory.getBytes(A(start+off),chunk,0,n))!=n:
                raise RuntimeError('Unmapped stage bytes')
            digest.update(bytes((int(x)&255) for x in chunk[:n]))
        if digest.hexdigest()!=expected:raise RuntimeError('Ghidra boot stage SHA mismatch')
    relocations=[]
    if candidate!='none':
        j=json.loads(Path(candidate).read_text())
        row=j['data'][role]
        if row.get('sha256')!=expected or row.get('status')!='ok':
            raise RuntimeError('Relocation candidates not owner verified')
        relocations=row.get('function_candidates',[])[:200]
    recovered=[]
    for c in relocations:
        v=int(c['candidate_vaddr'],16)
        entry=image_base+v
        aa=A(entry)
        block=memory.getBlock(aa)
        if block is None or not block.isExecute():continue
        fn=fm.getFunctionContaining(aa)
        if fn is None:
            try:
                disassemble(aa)
                fn=createFunction(aa,'mpfree_v10_recovered_'+format(v,'x'))
            except Exception as e:
                recovered.append({'at':hex(entry),'error':str(e)[:180]})
                continue
        if fn:recovered.append({'at':str(fn.getEntryPoint()),'name':str(fn.getName()),
                                'slot':c['relocation_slot']})
    targets=set()
    for v in ADDRESS[role]:
        fn=fm.getFunctionContaining(A(v))
        if fn:targets.add(str(fn.getEntryPoint()))
    for r in recovered:
        if r.get('at'):targets.add(r['at'])
    # Include one-hop callers and callees of pinned functions.
    snapshots={}
    for s in sorted(targets):
        fn=fm.getFunctionAt(A(int(s,16)))
        if fn is None:continue
        caller_set=set()
        for ref in rm.getReferencesTo(fn.getEntryPoint()):
            parent=fm.getFunctionContaining(ref.getFromAddress())
            if parent:caller_set.add(str(parent.getEntryPoint()))
        callee_set=set();computed=[]
        it=listing.getInstructions(fn.getBody(),True)
        while it.hasNext():
            ins=it.next()
            flow=ins.getFlowType()
            if flow.isCall():
                for ref in ins.getFlows():
                    callee=fm.getFunctionContaining(ref)
                    if callee:callee_set.add(str(callee.getEntryPoint()))
            if flow.isComputed() and len(computed)<60:
                computed.append({'address':str(ins.getAddress()),'instruction':str(ins),
                                 'type':str(flow)})
        snapshots[s]={'name':str(fn.getName()),'callers':sorted(caller_set)[:60],
                      'callees':sorted(callee_set)[:60],'computed':computed,
                      'bytes':int(fn.getBody().getNumAddresses())}
    for s,x in list(snapshots.items()):
        for near in x['callers'][:8]+x['callees'][:8]:
            if near not in snapshots:
                fn=fm.getFunctionAt(A(int(near,16)))
                if fn and fn.getBody().getNumAddresses()>24:
                    snapshots[near]={'name':str(fn.getName()),
                       'callers':[],'callees':[],'computed':[],
                       'bytes':int(fn.getBody().getNumAddresses()),'one_hop':True}
    # The V9 ELF relocation parser identified slots pointing into .text.
    # Inspect adjacent slots as an evidence graph, WITHOUT treating
    # pointers merely near one another as proof of a specific vtable.
    pointer_rows=[]
    for c in relocations:
        slot=image_base+int(c['relocation_slot'],16)
        entry={'slot_vaddr':c['relocation_slot'],
               'candidate_vaddr':c['candidate_vaddr'],
               'function_entry':hex(image_base+int(c['candidate_vaddr'],16)),
               'around':[]}
        for offset in range(-12,20,4):
            x=slot+offset
            block=memory.getBlock(A(x))
            if block is None or not block.isInitialized():continue
            try:
                val=int(memory.getInt(A(x)))&0xffffffff
                target=fm.getFunctionContaining(A(image_base+(val&~1)))
                entry['around'].append({'delta':offset,'address':hex(x),'word':hex(val),
                                        'possible_relative_function':str(target.getEntryPoint()) if target else None})
            except Exception:pass
        xrefs=[]
        for ref in rm.getReferencesTo(A(slot)):
            parent=fm.getFunctionContaining(ref.getFromAddress())
            xrefs.append({'from':str(ref.getFromAddress()),
                          'function':str(parent.getEntryPoint()) if parent else None})
            if len(xrefs)>=20:break
        entry['xrefs_to_slot']=xrefs
        pointer_rows.append(entry)
    decompiler=DecompInterface();decompiler.toggleCCode(True)
    decompiler.setSimplificationStyle('decompile')
    if not decompiler.openProgram(program):
        raise RuntimeError('Ghidra decompiler failed')
    limit={'az01-update':20,'libaz01-prog':150,'spl':35,'uboot':65}[role]
    ordered=sorted(snapshots,key=lambda k:(snapshots[k].get('one_hop',False),
                            k not in targets,k))
    decompiled=0
    try:
        for s in ordered[:limit]:
            fn=fm.getFunctionAt(A(int(s,16)))
            if fn is None:continue
            item=snapshots[s]
            try:
                dec=decompiler.decompileFunction(fn,60,monitor)
                if dec and dec.decompileCompleted() and dec.getDecompiledFunction():
                    raw=str(dec.getDecompiledFunction().getC())
                    raw=re.sub(r'"[^"\n]{120,}"','"[LONG STRING REDACTED]"',raw)
                    raw=re.sub(r'"[^"\n]*(?:password|private.key|secret|token|credential)[^"\n]*"',
                               '"[SENSITIVE LITERAL REDACTED]"',raw,flags=re.I)
                    item['pseudocode']=raw[:19000]
                    item['truncated']=len(raw)>19000
                    decompiled+=1
                else:item['error']=str(dec.getErrorMessage()) if dec else 'no decompilation'
            except Exception as e:item['error']=type(e).__name__+': '+str(e)[:180]
    finally:decompiler.dispose()
    report={'schema':'mpfree-v10-directed-static','role':role,
            'source_sha256_prechecked_by_launcher':expected,
            'boot_stage_memory_sha_verified':role in ('spl','uboot'),
            'image_base':hex(image_base),
            'recovered':recovered,'decompiled':decompiled,
            'functions':snapshots,'relocation_slot_neighbors':pointer_rows,
            'limitations':[
                'Function pointers near one another are NOT a validated backend vtable.',
                'Relative ELF relocations require consistent load-base interpretation.',
                'Ghidra pseudocode on ARM uses recovered approximations and may be wrong.',
                'No code, OTP, emmc, USB, ROM or MPC hardware was executed/written.',
                'ROM MaskROM entry, SD straps and physical recoverability remain unproven.']}
    dest.parent.mkdir(parents=True,exist_ok=True)
    old=os.umask(0o077)
    try:dest.write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    finally:os.umask(old)
    print('MPFree V10',role,'decomp',decompiled,'relocs',len(pointer_rows),'functions',len(snapshots))

mpfree_v10_run()
