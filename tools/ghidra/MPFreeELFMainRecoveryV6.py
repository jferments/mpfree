# @runtime PyGhidra
# @category MPFree
# RECOVER FUNCTIONS ONLY IN NEW THROWAWAY GHIDRA ELF PROJECTS.
# Owner-verified ELF pointer relocations are calculated by offline parser.
# Never run firmware code and never attach a debugger to the MPC.
import json
import os
import re
from pathlib import Path
from ghidra.app.decompiler import DecompInterface
from java.math import BigInteger

args=list(getScriptArgs())
if len(args)!=4:
    raise RuntimeError('Usage: /absolute/candidates.json /usr/path /absolute/report.json elfsha')
candidates=Path(str(args[0]));sourcepath=str(args[1]);out=Path(str(args[2]));expected=str(args[3])
if not candidates.is_absolute() or not out.is_absolute() or not re.fullmatch('[a-f0-9]{64}',expected):
    raise RuntimeError('Invalid V6 private arguments')
report=json.loads(candidates.read_text('utf-8'))
record=report['data'].get(sourcepath,{})
if record.get('status')!='ok' or record.get('sha256')!=expected:
    raise RuntimeError('ELF source mismatch - do not analyze')
p=currentProgram
if p is None or 'ELF' not in str(p.getExecutableFormat()).upper():
    raise RuntimeError('Must be imported ELF program')
if 'ARM:' not in str(p.getLanguageID()):
    raise RuntimeError('Must be ARM Ghidra project')
base=int(p.getImageBase().getOffset())
fm=p.getFunctionManager();mem=p.getMemory();ctx=p.getProgramContext()
space=p.getAddressFactory().getDefaultAddressSpace()
A=lambda v:space.getAddress(int(v))
monitor.setMessage('MPFree offline V6 ARM RELATIVE function recovery')
result={'schema':'mpfree-v6-relocation-function-recovery',
        'source_path':sourcepath,'sha256':expected,'ghidra_image_base':hex(base),
        'project_type':'disposable, never saved to original projects',
        'candidates':[],'found_new_functions':0,'successfully_decompiled':0,
        'limitations':[
          'A relocated pointer into .text is a candidate function, not confirmed main.',
          'ARM Thumb mode and reloc addend must be correct; Ghidra may still fail.',
          'Function creation modifies only temporary Ghidra analysis database.',
          'This script never executes the ELF or contacts hardware.',
        ]}
decompiler=DecompInterface()
decompiler.toggleCCode(True)
decompiler.setSimplificationStyle('decompile')
if not decompiler.openProgram(p):raise RuntimeError('Ghidra decompiler unavailable')
try:
    for row in record.get('function_candidates',[])[:120]:
        v=int(row['candidate_vaddr'],16)
        addr=A(base+v)
        b=mem.getBlock(addr)
        item=dict(row)
        item['ghidra_address']=str(addr)
        if b is None or not b.isExecute():
            item['status']='not executable in imported image'
            result['candidates'].append(item)
            continue
        fn=fm.getFunctionContaining(addr)
        if fn is None:
            try:
                tmode=ctx.getRegister('TMode')
                if bool(row.get('thumb')) and tmode is not None:
                    # The TMode register affects *Ghidra's temporary
                    # interpretation*, not any physical CPU register.
                    ctx.setValue(tmode,addr,addr,BigInteger.ONE)
                disassemble(addr)
                fn=createFunction(addr,'mpfree_candidate_'+format(v,'x'))
                if fn is not None:result['found_new_functions']+=1
            except Exception as error:
                item['create_error']=str(error)[:500]
        if fn is None:fn=fm.getFunctionContaining(addr)
        if fn is None:
            item['status']='not recoverable as function'
            result['candidates'].append(item)
            continue
        item['status']='function recovered or already existed'
        item['function_name']=str(fn.getName())
        item['function_entry']=str(fn.getEntryPoint())
        item['size']=int(fn.getBody().getNumAddresses())
        try:
            res=decompiler.decompileFunction(fn,60,monitor)
            if res and res.decompileCompleted() and res.getDecompiledFunction():
                src=str(res.getDecompiledFunction().getC())
                src=re.sub(r'"[^"\n]{80,}"','"[LONG LITERAL REDACTED]"',src)
                src=re.sub(r'"[^"\n]*(?:secret|password|private|token|serialno)[^"\n]*"',
                           '"[SENSITIVE LITERAL REDACTED]"',src,flags=re.I)
                item['pseudocode']=src[:26000]
                item['truncated']=len(src)>26000
                result['successfully_decompiled']+=1
            else:
                item['decompile_error']=str(res.getErrorMessage()) if res else 'no result'
        except Exception as error:item['decompile_error']=str(error)[:500]
        result['candidates'].append(item)
finally:decompiler.dispose()
out.parent.mkdir(parents=True,exist_ok=True)
old=os.umask(0o077)
try:out.write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
finally:os.umask(old)
print('MPFree V6 ELF relocation function recovery:',sourcepath,
      'tested candidates',len(result['candidates']),
      'new functions',result['found_new_functions'],
      'decompiled',result['successfully_decompiled'])