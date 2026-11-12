# @runtime PyGhidra
# @category MPFree
"""MPFree V16: independently verify owner image in saved read-only Ghidra
project, export disposable private P3 bytes for isolated ARM tests, and
trace selected U-Boot and SPL functions with full high P-code. No MPC I/O.
"""
import hashlib,json,os
from pathlib import Path
from datetime import datetime,timezone
from ghidra.app.decompiler import DecompInterface
from jpype import JArray,JByte

IMAGES={
  'p1':(0xff704000,32768,'6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f'),
  'p3':(0x00100000,363649,'b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a')}
DOMAINS={
 'p1':{
  'rom_loader':[0xff704004,0xff704060,0xff705208,0xff7052c0,0xff705310],
  'mmc_sd':[0xff705350,0xff705398,0xff7053c2,0xff7054a4,0xff708f98],
  'recovery_and_ddr':[0xff705316,0xff7056a8,0xff708b74,0xff708cdc,0xff707f48]},
 'p3':{
  'auth_and_oem':[0x00122834,0x00122854,0x0012dc78,0x00122bf8,0x00122c5c],
  'usb_download_receive':[0x00122980,0x00122a10,0x0012dda2,0x0012247c,0x00121dd0],
  'ram_boot_image':[0x00122964,0x001226b4,0x0010e390,0x0010a6a4,0x00127a1c,0x00101744],
  'command_and_external_media':[0x00110484,0x001095f8,0x0010b74c,0x00108560,0x00100fe4,0x001011e0],
  'recovery_and_reset':[0x0010d14c,0x0011622a,0x0010e000,0x0010f388],
 }}
args=list(getScriptArgs())
if len(args)!=3 or str(args[0]) not in IMAGES:raise RuntimeError('Usage: p1|p3 absolute-report.json private-export.raw|none')
role=str(args[0]);out=Path(str(args[1]));export=str(args[2]);base,size,want=IMAGES[role]
if not out.is_absolute() or (export!='none' and (role!='p3' or not Path(export).is_absolute())):raise RuntimeError('Absolute paths required')
p=currentProgram
if p is None or str(p.getLanguageID())!='ARM:LE:32:v7':raise RuntimeError('ARM v7 project required')
m=p.getMemory();space=p.getAddressFactory().getDefaultAddressSpace()
def A(n):return space.getAddress(int(n))
def H(n):return '0x%08x'%int(n)
block=m.getBlock(A(base))
if block is None or block.getStart().getOffset()!=base or block.getSize()!=size:raise RuntimeError('Unexpected owner memory-image layout')
raw=bytearray();buf=JArray(JByte)(32768)
for i in range(0,size,32768):
 n=min(32768,size-i)
 if int(m.getBytes(A(base+i),buf,0,n))!=n:raise RuntimeError('Image read truncated')
 raw.extend(int(x)&255 for x in buf[:n])
raw=bytes(raw)
if hashlib.sha256(raw).hexdigest()!=want:raise RuntimeError('Hash mismatch against exact saved owner firmware')
if export!='none':
 path=Path(export)
 if path.is_symlink() or path.exists():raise RuntimeError('Private export must be a new regular file')
 path.parent.mkdir(parents=True,exist_ok=True)
 mask=os.umask(0o077)
 try:
  with path.open('xb') as f:f.write(raw)
  os.chmod(str(path),0o600)
 finally:os.umask(mask)
fm=p.getFunctionManager();listing=p.getListing();refm=p.getReferenceManager()
dec=DecompInterface();dec.toggleCCode(True);dec.setSimplificationStyle('decompile')
if not dec.openProgram(p):raise RuntimeError('Decompiler cannot open verified project')
entries={};domains={}
for domain,addresses in DOMAINS[role].items():
 domain_funcs=[]
 for address in addresses:
  fn=fm.getFunctionContaining(A(address))
  if fn:
   start=int(fn.getEntryPoint().getOffset());domain_funcs.append(H(start));entries[start]=fn
 domains[domain]=sorted(set(domain_funcs))
result={'schema':'mpfree-v16-real-image-static','role':role,'owner_sha256':want,'hash_verified':True,
 'private_export':export!='none','date_utc':datetime.now(timezone.utc).isoformat(),
 'domains':domains,'functions':{},'global_refs':{},'limitations':[
 'Saved Ghidra project and its decompilation are STATIC evidence, not hardware proof.',
 'No live USB, eMMC, vendor updater, or original U-Boot entrypoint was launched.',
 'Private full binary export is deleted after guest tests; never archived.']}
watch=[0x0014d690,0x0014d680,0x0014d684,0x0014d658,0xff730094,0xff7300a0] if role=='p3' else [0xff730094,0xff7300a0]
for address in watch:
 found=[]
 for x in refm.getReferencesTo(A(address)):
  src=int(x.getFromAddress().getOffset());fn=fm.getFunctionContaining(A(src))
  found.append({'site':H(src),'function':H(fn.getEntryPoint().getOffset()) if fn else None,'type':str(x.getReferenceType())})
  if len(found)>=300:break
 result['global_refs'][H(address)]=found
try:
 for start,fn in sorted(entries.items()):
  record={'name':str(fn.getName()),'entry':H(start),'calls':[],'branch_sites':[],'pcode':[]}
  it=listing.getInstructions(fn.getBody(),True)
  while it.hasNext():
   ins=it.next();ft=ins.getFlowType()
   if ft.isCall():record['calls'].append({'site':str(ins.getAddress()),'targets':[str(t) for t in ins.getFlows()]})
   if ft.isConditional():record['branch_sites'].append({'site':str(ins.getAddress()),'opcode':str(ins)[:180],'targets':[str(t) for t in ins.getFlows()]})
  try:
   r=dec.decompileFunction(fn,60,monitor)
   if r and r.decompileCompleted() and r.getDecompiledFunction():
    import re
    c=str(r.getDecompiledFunction().getC())
    c=re.sub(r'(?i)inmusic-unlock-magic-[0-9a-f]+','inmusic-unlock-magic-[REDACTED]',c)
    record['pseudocode']=c[:30000]
    record['decompile_truncated']=len(c)>30000
    high=r.getHighFunction()
    if high is not None:
     ops=high.getPcodeOps()
     while ops.hasNext() and len(record['pcode'])<4000:
      op=ops.next();action=op.getMnemonic()
      if action not in ('CALL','CALLIND','LOAD','STORE','CBRANCH','BRANCH','INT_LESS','INT_LESSEQUAL','INT_EQUAL','INT_NOTEQUAL','INT_ADD','INT_SUB','INT_ZEXT','INT_CARRY','INT_SLESS'):continue
      record['pcode'].append({'site':str(op.getSeqnum().getTarget()),'opcode':action,
               'operands':[str(op.getInput(i))[:110] for i in range(op.getNumInputs())]})
   else:record['error']=str(r.getErrorMessage()) if r else 'no decompilation'
  except Exception as e:record['error']=str(e)[:350]
  result['functions'][H(start)]=record
finally:dec.dispose()
result['statistics']={'functions':len(result['functions']),
 'high_pcode_ops':sum(len(x['pcode']) for x in result['functions'].values()),
 'branch_sites':sum(len(x['branch_sites']) for x in result['functions'].values()),
 'source_bytes_verified':len(raw)}
out.parent.mkdir(parents=True,exist_ok=True);mask=os.umask(0o077)
try:out.write_text(json.dumps(result,indent=2)+'\n',encoding='utf8')
finally:os.umask(mask)
print('MPFree V16',role,'real owner image verified, domains',len(domains),'functions',len(entries),'pcode',result['statistics']['high_pcode_ops'])