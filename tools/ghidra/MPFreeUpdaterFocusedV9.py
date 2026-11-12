# @runtime PyGhidra
# @category MPFree
# Static V9: updater entrypoints, backend callbacks, RK3288 ROM and U-Boot.
# Never executes ELF code or sends data to hardware. Reports are private.
import hashlib
import json
import os
import re
from pathlib import Path
from ghidra.app.decompiler import DecompInterface

SHA={
 'az01-update':'57511e1d503f3ceaae3cb9bc40174e073b7ae410e65c986f1991731c77ae655b',
 'libaz01-prog':'af613a88f1b089069ed81a5b5fdf1ceb8c8f68bcd43c7d26b31c9ec9590e4623',
 'spl':'6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f',
 'uboot':'b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a'
}
ANCHORS={
 'az01-update':[],
 'libaz01-prog':[0x1738c,0x17554,0x1798c,0x14b98,0x14ac8,
                 0x14c50,0x14d00,0x1e194,0x1f094,0x1b9bc,
                 0x14154,0x166f8,0x13828,0x1f968,0x1f990,
                 0x1f9c8,0x1f9fc,0x1fdc4,0x1fd60],
 'spl':[0xff704004,0xff705310,0xff705350,0xff7052c0,0xff7054a4],
 'uboot':[0x00100fe4,0x00101744,0x00122c5c,0x00122bf8,0x00122834,
          0x00122980,0x001226b4,0x001011e0]
}
PAT=re.compile(r'rockusb|bootrom|maskrom|brom|kexec|usb|program.fuses|'
               r'write.boot|bootloader|flash|erase|partition|rollback|'
               r'signature|verify|digest|update|reboot|usb.update|'
               r'bootcmd|spl|mmc|image|compatib|loadaddr|oem',re.I)
PRIVATE=re.compile(r'private.?key|password|credential|secret|token|serial.?number|ethaddr',re.I)

def mpfree_v9_run():
    args=list(getScriptArgs())
    if len(args)!=4:raise RuntimeError('role output-json verified-sha candidates-json-or-dash')
    role,dest,sha,candidate_arg=str(args[0]),Path(str(args[1])),str(args[2]),str(args[3])
    if role not in SHA or sha!=SHA[role] or not dest.is_absolute():
        raise RuntimeError('Unknown role or source SHA mismatch')
    p=currentProgram
    if p is None or 'ARM:' not in str(p.getLanguageID()):
        raise RuntimeError('Expected ARM Ghidra program')
    base=int(p.getImageBase().getOffset())
    space=p.getAddressFactory().getDefaultAddressSpace()
    def addr(v):return space.getAddress(int(v))
    fm=p.getFunctionManager();ls=p.getListing();refs=p.getReferenceManager();mem=p.getMemory()
    # Existing saved boot stages are checked against the actual program
    # memory, not just against the backup file on disk.
    if role in ('spl','uboot'):
        from jpype import JArray,JByte
        start,size=((0xff704000,32768) if role=='spl' else (0x00100000,363649))
        block=mem.getBlock(addr(start))
        if not block or block.getStart().getOffset()!=start or block.getSize()!=size:
            raise RuntimeError('Unexpected P1/P3 memory block')
        buf=JArray(JByte)(32768);h=hashlib.sha256()
        for off in range(0,size,32768):
            n=min(32768,size-off)
            if int(mem.getBytes(addr(start+off),buf,0,n))!=n:
                raise RuntimeError('Unreadable stage memory')
            h.update(bytes(int(x)&255 for x in buf[:n]))
        if h.hexdigest()!=sha:raise RuntimeError('Ghidra boot-stage bytes not owner verified')
    recovered=[]
    if candidate_arg!='-':
        doc=json.loads(Path(candidate_arg).read_text(encoding='utf-8'))
        entry=doc['data'].get(role,{})
        if entry.get('sha256')!=sha or entry.get('status')!='ok':
            raise RuntimeError('Candidate source SHA mismatch')
        for row in entry.get('function_candidates',[])[:160]:
            v=int(row['candidate_vaddr'],16)
            va=base+v
            a=addr(va)
            block=mem.getBlock(a)
            status={'candidate_vaddr':hex(v),'thumb':bool(row.get('thumb')),'slot':row.get('relocation_slot')}
            if not block or not block.isExecute():
                status['status']='not executable';recovered.append(status);continue
            fn=fm.getFunctionContaining(a)
            if fn is None:
                try:
                    # Only a private disposable Ghidra project may be changed.
                    # createFunction/disassemble do not touch the ELF on disk.
                    disassemble(a)
                    fn=createFunction(a,'mpfree_v9_candidate_'+format(v,'x'))
                    status['created']=bool(fn)
                except Exception as e:status['error']=type(e).__name__+': '+str(e)[:200]
            if fn is None:fn=fm.getFunctionContaining(a)
            status['entry']=str(fn.getEntryPoint()) if fn else None
            recovered.append(status)
    funcs={};iteration=fm.getFunctions(True)
    while iteration.hasNext():
        fn=iteration.next();funcs[str(fn.getEntryPoint())]=fn
    callers={};callees={};indirect={}
    for k,fn in funcs.items():
        callers[k]=set();callees[k]=set();indirect[k]=[]
    for k,fn in funcs.items():
        it=ls.getInstructions(fn.getBody(),True)
        while it.hasNext():
            ins=it.next();typ=ins.getFlowType()
            if not (typ.isCall() or typ.isJump()):continue
            targets=[str(x) for x in ins.getFlows()]
            if typ.isCall() and targets:
                for target in targets:
                    callees[k].add(target)
                    if target in callers:callers[target].add(k)
            elif (typ.isCall() or typ.isComputed()) and not targets and len(indirect[k])<70:
                indirect[k].append({'at':str(ins.getAddress()),'mnemonic':str(ins.getMnemonicString()),
                                    'kind':str(typ)})
    strings=[];score={k:0 for k in funcs};groups={k:set() for k in funcs}
    it=ls.getDefinedData(True)
    while it.hasNext() and len(strings)<1300:
        d=it.next()
        try:
            raw=str(d.getValue())
            if not 4<=len(raw)<=900 or not PAT.search(raw):continue
            val='[REDACTED]' if PRIVATE.search(raw) else raw[:190]
            xrefs=[]
            for ref in refs.getReferencesTo(d.getAddress()):
                fn=fm.getFunctionContaining(ref.getFromAddress())
                if fn is None:continue
                key=str(fn.getEntryPoint())
                score[key]=score.get(key,0)+18
                groups.setdefault(key,set()).add('string_reference')
                if len(xrefs)<70:xrefs.append({'at':str(ref.getFromAddress()),'caller':key})
            strings.append({'at':str(d.getAddress()),'text':val,'xrefs':xrefs})
        except Exception:continue
    for k,fn in funcs.items():
        score[k]+=min(20,len(callers.get(k,set())))*2+min(20,len(callees[k]))
        score[k]+=min(10,len(indirect[k]))*10
        if PAT.search(str(fn.getName())):
            score[k]+=80;groups[k].add('name')
        if fn.getBody().getNumAddresses()<=16:
            score[k]-=25  # prioritize real application code above PLT stubs
    for v in ANCHORS[role]:
        a=addr(v)
        f=fm.getFunctionContaining(a)
        if f is None:continue
        k=str(f.getEntryPoint());score[k]+=300;groups[k].add('verified_static_anchor')
    for c in recovered:
        if c.get('entry') in score:
            score[c['entry']]+=270
            groups[c['entry']].add('relocation_function_candidate')
    anchors=sorted(funcs,key=lambda k:(-score.get(k,0),k))[:70]
    for k in anchors:
        for other in callers.get(k,set())|callees.get(k,set()):
            if other in score:
                score[other]+=15;groups[other].add('caller_or_callee')
    limit={'az01-update':180,'libaz01-prog':290,'spl':100,'uboot':145}[role]
    chosen=sorted(funcs,key=lambda k:(-score.get(k,0),k))[:limit]
    dec=DecompInterface();dec.toggleCCode(True)
    dec.setSimplificationStyle('decompile')
    if not dec.openProgram(p):raise RuntimeError('Cannot initialize decompiler')
    details={}
    try:
        for k in chosen:
            fn=funcs[k]
            entry={'name':str(fn.getName()),'size':int(fn.getBody().getNumAddresses()),
                   'groups':sorted(groups[k]),'score':score.get(k,0),
                   'callers':sorted(callers.get(k,[]))[:75],
                   'callees':sorted(callees[k])[:100],
                   'indirect':indirect[k]}
            try:
                de=dec.decompileFunction(fn,55,monitor)
                if de and de.decompileCompleted() and de.getDecompiledFunction():
                    code=str(de.getDecompiledFunction().getC())
                    code=re.sub(r'"[^"\n]{120,}"','"[LONG LITERAL REDACTED]"',code)
                    code=re.sub(r'"[^"\n]*(?:private.?key|password|secret|token)[^"\n]*"',
                                '"[SENSITIVE REDACTED]"',code,flags=re.I)
                    entry['pseudocode']=code[:15000]
                    entry['truncated']=len(code)>15000
                else:entry['error']=str(de.getErrorMessage()) if de else 'no result'
            except Exception as e:entry['error']=type(e).__name__+': '+str(e)[:200]
            details[k]=entry
    finally:dec.dispose()
    report={
        'schema':'mpfree-v9-updater-backend-and-rom-static',
        'role':role,'source_sha256_verified_offline':sha,'image_base':hex(base),
        'recognized_functions':len(funcs),'recovered_relocation_candidates':recovered,
        'decompiled':sum('pseudocode' in x for x in details.values()),
        'function_details':details,'strings_with_xrefs':strings,
        'computed_sites':{k:v for k,v in indirect.items() if v},
        'unresolved_hardware_gates':[
         'Native Rockchip BootROM mode and USB wiring NOT verified by these results.',
         'Which backend vtable is selected at runtime remains to be traced.',
         'Ghidra direct calls do not capture all callbacks, GOT references or jump tables.',
         'Software presence of loader/flash does not demonstrate safe eMMC recovery.',
         'No Akai binary executed and no physical device or OTP touched.'],
    }
    dest.parent.mkdir(parents=True,exist_ok=True)
    mask=os.umask(0o077)
    try:dest.write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    finally:os.umask(mask)
    print('MPFree V9',role,'functions',len(funcs),
          'candidate pointers',len(recovered),'details',len(details),
          'strings',len(strings),'output',dest)

mpfree_v9_run()
