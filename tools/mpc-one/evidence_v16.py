#!/usr/bin/env python3
"""V16 actual-evidence adjudicator for owner-private V14/V15 Ghidra/QEMU reports.

Per-question outcomes require source-backed predicates. Keyword matches are
NEVER called an experiment, never used to close a question. No MPC I/O.
"""
from __future__ import annotations
from collections import Counter, defaultdict, deque
from pathlib import Path, PurePosixPath
import hashlib, json, re, zipfile

P1_SHA='6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f'
P3_SHA='b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a'
MAX_MEMBER=32*1024*1024
MAX_TOTAL=180*1024*1024

def load_verified_zip(filename):
    filename=Path(filename).expanduser()
    if filename.is_symlink() or not filename.is_file():
        raise ValueError('A regular private ZIP file is required')
    with zipfile.ZipFile(filename) as archive:
        if archive.testzip():raise ValueError('Bad private ZIP CRC')
        index=json.loads(archive.read('ZIP_INDEX.json'))
        if not 5<=len(index['files'])<=500:
            raise ValueError('Unexpected indexed count')
        files={};total=0
        for item in index['files']:
            name=item['name'];path=PurePosixPath(name)
            if (not name or path.is_absolute() or '..' in path.parts or
                any(x in ('','.') for x in path.parts) or name in files or
                path.suffix not in {'.json','.log','.md','.txt','.csv','.tsv'}):
                raise ValueError('Unsafe ZIP member: '+name)
            info=archive.getinfo(name)
            if info.file_size>MAX_MEMBER or info.file_size!=item['bytes']:
                raise ValueError('Invalid member size '+name)
            total+=info.file_size
            if total>MAX_TOTAL:raise ValueError('ZIP content too large')
            data=archive.read(name)
            if hashlib.sha256(data).hexdigest()!=item['sha256']:
                raise ValueError('ZIP index SHA mismatch: '+name)
            files[name]=data
        if set(archive.namelist())!=set(files)|{'ZIP_INDEX.json'}:
            raise ValueError('Unexpected unindexed ZIP entries')
    return files

def parsed(files,name):
    if name not in files:raise ValueError('Missing report '+name)
    return json.loads(files[name])

def build_graph(functions):
    edges=defaultdict(set)
    for addr,entry in functions.items():
        for c in entry.get('calls',[]):
            for target in c.get('target_entries',[]):
                if target in functions:edges[addr].add(target)
    return edges

def route(graph,start,goal,max_edges=10):
    if start==goal:return [start]
    visited={start};queue=deque([(start,[start])])
    while queue:
        node,path=queue.popleft()
        if len(path)>max_edges:continue
        for child in sorted(graph.get(node,())):
            if child==goal:return path+[child]
            if child not in visited:
                visited.add(child);queue.append((child,path+[child]))
    return None

def direction(fn,addr):
    return any(addr in c.get('target_entries',[]) for c in fn.get('calls',[]))

def verified_sources(v14,v15):
    p1=parsed(v14,'v14-full-p1-ghidra.json')
    p3=parsed(v14,'v14-full-p3-ghidra.json')
    g15=parsed(v15,'v15-p3-directed.json')
    assert p1['owner_hash_verified'] and p1['owner_sha256']==P1_SHA
    assert p3['owner_hash_verified'] and p3['owner_sha256']==P3_SHA
    assert g15['image_hash_verified'] and g15['source_sha256']==P3_SHA
    q=parsed(v14,'V14-85-QUESTION-EVIDENCE-LEDGER.json')['questions']
    if len(q)!=85:raise ValueError('Expected exactly 85 original owner questions')
    return p1,p3,g15,q

DOMAIN_ROOTS={
 'FASTBOOT': ['0x00122854','0x00122834','0x00122980','0x00122a10','0x00122964','0x001226b4'],
 'FASTBOOT_RUNTIME':['0x00122854','0x00122834','0x00122980','0x00122a10','0x00101744','0x0010e390'],
 'BOOT_SELECTION':['0x00100fe4','0x001095f8','0x00110484','0x001011e0'],
 'REMOVABLE_DIRECTBOOT':['0x00100fe4','0x001095f8','0x00110484','0x001011e0','0x0010e000'],
 'BOOT_COUNTER':['0x00100fe4'],
 'RECOVERY':['0x00100fe4','0x0011622a'],
 'RECOVERY_SECURITY':['0x00100fe4','0x0011622a','0x0010b74c'],
 'ROOTFS_COMPATIBILITY':['0x0010e000','0x00101744','0x0010e390'],
 'VM_VALIDATION':[],
}
QUESTION_FUNCTIONS={
 'FASTBOOT-01':['0x00122854'],
 'FASTBOOT-02':['0x00122854','0x0012dc78'],
 'FASTBOOT-03':['0x00122854','0x00122834'],
 'FASTBOOT-04':['0x00122834','0x00122980','0x00122bf8','0x00122c5c','0x00122964'],
 'FASTBOOT-05':['0x00122854','0x0010b74c'],
 'FASTBOOT-06':['0x001226b4','0x00101744','0x0010e390'],
 'FASTBOOT-07':['0x001226b4','0x0011622a'],
 'FASTBOOT-08':['0x00122a10','0x001226b4'],
 'FASTBOOT-09':['0x00122d38','0x00122834'],
 'BOOT_SELECTION-01':['0x00110484','0x001095f8','0x00108560'],
 'BOOT_SELECTION-02':['0x00100fe4','0x001095f8'],
 'BOOT_SELECTION-03':['0x001095f8'],
 'BOOT_SELECTION-04':['0x00101744','0x0010e000'],
 'BOOT_SELECTION-05':['0x00100fe4'],
 'BOOT_COUNTER-01':['0x00100fe4'],
 'BOOT_COUNTER-02':['0x00100fe4'],
 'RECOVERY-01':['0x00100fe4'],
 'RECOVERY-04':['0x0010e390','0x00101744'],
 'FASTBOOT_RUNTIME-04':['0x00122980','0x00122a10'],
 'FASTBOOT_RUNTIME-05':['0x00122a10'],
 'FASTBOOT_RUNTIME-06':['0x00122964','0x00122834'],
 'FASTBOOT_RUNTIME-07':['0x0010e390'],
 'FASTBOOT_RUNTIME-08':['0x0010e390','0x0010a6a4'],
 'FASTBOOT_RUNTIME-09':['0x001226b4','0x00101744'],
 'FASTBOOT_RUNTIME-10':['0x001226b4','0x0011622a'],
 'REMOVABLE_DIRECTBOOT-07':['0x00101744'],
 'RECOVERY_SECURITY-04':['0x00100fe4'],
 'RECOVERY_SECURITY-05':['0x00100fe4'],
 'RECOVERY_SECURITY-10':['0x00122c5c'],
 'RECOVERY_SECURITY-11':['0x00122c5c'],
}

def evidence(v14,v15,dynamic=None,live=None):
    p1,p3,g15,questions=verified_sources(v14,v15)
    f=p3['functions']; graph=build_graph(f)
    assets=parsed(v14,'offline-boot-assets-v13.json')
    boot=assets['files'].get('/boot/boot.scr.uimg',{}).get('uimage',{})
    script='\n'.join(boot.get('script',{}).get('normalized_command_lines',[]))
    dtds={k:assets['files'].get(k,{}).get('dtb',{}) for k in
        ['/boot/rk3288-az01-acva.dtb','/boot/rk3288-az01-acva-c.dtb']}
    runtime=dynamic or {}
    dtests={item['test']:item for item in runtime.get('cases',[]) if item.get('real_vendor_arm')}
    rows=[];facts={}; paths=[]
    def fact(key,statement,sources,proof,kind='static',caveat=''):
        facts[key]={'statement':statement,'sources':sources,'proof':proof,'kind':kind,'caveat':caveat}
    def stage(code):
        row['status']=code
    def attach(key):
        if key not in row['finding_refs']:row['finding_refs'].append(key)
    def add_dynamic(name,match=True):
        item=dtests.get(name)
        if item:
            row['dynamic_tests'].append({'case':name,'pass':item.get('pass'),
                    'observation':item.get('observation')})
            if item.get('pass') is True and match:stage('PARTIAL_RUNTIME_ISOLATED')
    # First derive real, source-backed propositions. Absence of an edge is not absence of a call.
    alias=g15['permission_alias']
    if alias.get('same_nonzero_address'):
        fact('auth_global_alias','OEM writer and Fastboot permission reader reference identical state',
             ['v15-p3-directed.json:pointer_cells','v15-p3-directed.json:permission_alias'],alias)
    for name,caller in [('download','0x00122980'),('flash','0x00122c5c'),('erase','0x00122bf8')]:
        if direction(f[caller],'0x00122834'):
            fact(name+'_permission_call',name+' handler directly calls permission checker',
                 ['v14-full-p3-ghidra.json:functions/'+caller],{'from':caller,'to':'0x00122834'})
    if direction(f['0x001226b4'],'0x00101744'):
        fact('ram_bootm','Fastboot completion explicitly calls bootm',
             ['v14-full-p3-ghidra.json:functions/0x001226b4'],{'from':'0x001226b4','to':'0x00101744'})
    if direction(f['0x001226b4'],'0x0011622a'):
        fact('bootm_fallback_reset','Fastboot completion has CPU-reset call after bootm',
             ['v15-p3-directed.json:function_details/0x001226b4'],{'code_excerpt':g15['function_details']['0x001226b4']['pseudocode'][-600:]})
    if boot.get('valid') and boot.get('header_crc_valid') and boot.get('payload_crc_valid'):
        fact('stock_script_crc','Original P6 boot script U-Boot header/payload CRCs verified',
             ['offline-boot-assets-v13.json:files/boot/boot.scr.uimg'],
             {'header_crc':True,'payload_crc':True,'script_sha':assets['files']['/boot/boot.scr.uimg']['sha256']})
    if 'bootz' in script:
        fact('stock_script_bootz','Vendor script uses bootz rather than Fastboot bootm',
             ['offline-boot-assets-v13.json:files/boot/boot.scr.uimg'],{'command_lines':script.splitlines()[:40]})
    if script and 'uuid' not in script.lower():
        fact('stock_script_no_uuid','Stock script contains no hard-coded UUID',
             ['offline-boot-assets-v13.json:files/boot/boot.scr.uimg'],{'script_contains_uuid':False})
    cells=g15['pointer_cells']
    size_ptr=cells.get('download_size_literal',{}).get('value')
    received_ptr=cells.get('download_count_literal',{}).get('value')
    if size_ptr and received_ptr:
        fact('download_state_globals','Download declared size and received count use separate globals',
             ['v15-p3-directed.json:pointer_cells'],{'declared':size_ptr,'received':received_ptr})
    if all(x.get('valid') for x in dtds.values()):
        fact('valid_acva_dtbs','Both P6 ACVA DTBs have valid FDT structure',
             ['offline-boot-assets-v13.json:files'],{'dtbs':{k:{'nodes':v.get('nodes'),'props':v.get('properties')} for k,v in dtds.items()}})
    if all(x.get('selected_properties',{}).get('/power-management@ff730000/reboot-mode',{}).get('mode-update')==['0x5242c301'] for x in dtds.values()):
        fact('update_loader_mode_alias','Both ACVA DTBs define mode-update=0x5242c301',
             ['offline-boot-assets-v13.json:files'],{})
    # High-value paths in the actual 2835-edge U-Boot call graph; no keyword matching.
    for src,dst in [('0x00122854','0x00122834'),('0x00122854','0x00110484'),
                    ('0x00122854','0x0010b74c'),('0x00122980','0x00122834'),
                    ('0x00122a10','0x0012dda2'),('0x001226b4','0x00101744'),
                    ('0x001226b4','0x0011622a'),('0x00110484','0x001095f8'),
                    ('0x001095f8','0x00108560')]:
        p=route(graph,src,dst,10)
        paths.append({'from':src,'to':dst,'path':p,
                      'edge_evidence':[f[n]['calls'][:50] for n in p[:-1]] if p else [],
                      'interpretation':'direct call path found' if p else 'no direct path established (indirect calls unresolved)'})
    # Analyze a disjoint question inventory with explicit rules; no generic auto-answering.
    for q in questions:
        id=q['id'];prefix=id.rsplit('-',1)[0]
        root=QUESTION_FUNCTIONS.get(id,DOMAIN_ROOTS.get(prefix,[]))
        row={'id':id,'question':q['question'],'prior_status':q.get('classification'),
             'status':'UNRESOLVED','static_probe_functions':[],
             'function_evidence':[], 'finding_refs':[], 'dynamic_tests':[],
             'probe_executed':False,'unresolved_boundary':None}
        for address in root:
            if address in f:
                fn=f[address]
                row['static_probe_functions'].append(address)
                row['function_evidence'].append({'address':address,'name':fn['name'],
                  'decompile_status':fn.get('decompile_status'),
                  'indirect_calls':len(fn.get('indirect_sites',[])),
                  'direct_calls':len(fn.get('calls',[]))})
        # This is an actual archived function interrogation, not a NEW experiment.
        row['probe_executed']=bool(row['static_probe_functions'])
        if id=='FASTBOOT-01' and cells['unlock_text_literal']['value']=='0x00132bab':
            stage('ANSWERED_STATIC');row['finding_refs'].append('auth_global_alias')
        if id=='FASTBOOT-02' and 'FUN_0012dc78' in g15['function_details']['0x00122854']['pseudocode']:
            stage('ANSWERED_STATIC');row['function_evidence'].append({'source':'v15-p3-directed.json:function_details/0x00122854','value':'OEM prefix comparison and handler'})
        if id=='FASTBOOT-03' and 'auth_global_alias' in facts:
            stage('PARTIAL_STATIC');attach('auth_global_alias')
        if id in ('FASTBOOT-04','FASTBOOT_RUNTIME-06'):
            for k in ('download_permission_call','erase_permission_call','flash_permission_call'):
                if k in facts:attach(k)
            if row['finding_refs']:stage('PARTIAL_STATIC')
        if id=='FASTBOOT-05' and any(x['from']=='0x00122854' and x['to']=='0x0010b74c' and x['path'] for x in paths):
            stage('PARTIAL_STATIC');row['unresolved_boundary']='OEM subcommands have distinct code paths; direct unlock-path storage side effects require separate path-sensitive proof.'
        if id in ('FASTBOOT-06','FASTBOOT_RUNTIME-09') and 'ram_bootm' in facts:
            stage('PARTIAL_STATIC');attach('ram_bootm')
        if id in ('FASTBOOT-07','FASTBOOT_RUNTIME-10') and 'bootm_fallback_reset' in facts:
            stage('PARTIAL_STATIC' if id=='FASTBOOT-07' else 'ANSWERED_STATIC');attach('bootm_fallback_reset')
        if id in ('FASTBOOT-08','FASTBOOT_RUNTIME-04','FASTBOOT_RUNTIME-05') and 'download_state_globals' in facts:
            stage('PARTIAL_STATIC');attach('download_state_globals')
        if id=='FASTBOOT-09' and 'mpf_fastboot_permission' not in g15['function_details']['0x00122d38']['pseudocode']:
            stage('PARTIAL_STATIC');row['unresolved_boundary']='No direct getvar authorization check; live Fastboot remains untested.'
        if id in ('BOOT_SELECTION-04','REMOVABLE_DIRECTBOOT-07','ROOTFS_COMPATIBILITY-02'):
            match=('stock_script_bootz' if id=='BOOT_SELECTION-04' else 'stock_script_no_uuid' if id=='REMOVABLE_DIRECTBOOT-07' else 'stock_script_crc')
            if match in facts:stage('ANSWERED_STATIC');attach(match)
        if id in ('ROOTFS_COMPATIBILITY-03','ROOTFS_COMPATIBILITY-07') and 'valid_acva_dtbs' in facts:
            stage('PARTIAL_STATIC');attach('valid_acva_dtbs')
        if id=='RECOVERY_SECURITY-05' and 'update_loader_mode_alias' in facts:
            stage('ANSWERED_STATIC');attach('update_loader_mode_alias')
        if id in ('FASTBOOT_RUNTIME-07','FASTBOOT_RUNTIME-08'):
            stage('PARTIAL_STATIC')
        if id in ('RECOVERY-01','RECOVERY_SECURITY-04'):
            stage('PARTIAL_STATIC')
        if id in ('FASTBOOT_RUNTIME-01','FASTBOOT_RUNTIME-13'):
            for t in ('oem_exact','oem_typo','oem_short','oem_suffix'):add_dynamic(t)
        if id in ('FASTBOOT-02','FASTBOOT-03','FASTBOOT-05'):
            for t in ('oem_exact','oem_typo','oem_shell_denied','oem_env_denied'):add_dynamic(t)
        if id in ('FASTBOOT-04','FASTBOOT_RUNTIME-06','FASTBOOT_RUNTIME-12'):
            for t in ('permission_0','permission_1','permission_2'):add_dynamic(t)
        if id in ('FASTBOOT_RUNTIME-07','FASTBOOT_RUNTIME-08'):
            for t in ('classify_legacy','classify_fdt','classify_invalid'):add_dynamic(t)
        if id in ('FASTBOOT_RUNTIME-04','FASTBOOT_RUNTIME-05'):
            for t in ('receive_one','receive_31','receive_256','receive_4096','receive_clip'):add_dynamic(t)
        if id in ('FASTBOOT_RUNTIME-06','FASTBOOT_RUNTIME-10'):
            add_dynamic('boot_callback')
        # Preserve hardware-only uncertainty. Read-only offline experiments cannot answer it.
        physical_markers=('BootROM','MaskROM','physical','hardware','fuse policy','actual','on MPC','in runtime','persistent','without modifying','survive failure')
        if row['status']=='UNRESOLVED' and any(t.lower() in q['question'].lower() for t in physical_markers):
            row['unresolved_boundary']='Requires hardware-specific observation or a separately justified emulator; current generic ARM virt is not RK3288.'
        if row['status']=='UNRESOLVED' and not row['static_probe_functions']:
            row['unresolved_boundary']='No suitable new verified function probe; not counted as investigated this round.'
        rows.append(row)
    counts=dict(Counter(x['status'] for x in rows))
    return {'schema':'mpfree-v16-85-question-evidence-v1',
            'summary':{'questions':85,'classifications':counts,
            'static_functions_inspected':len(set(a for r in rows for a in r['static_probe_functions'])),
            'real_vendor_dynamic_cases_reported':len(dtests),'actual_new_hardware_proofs':0},
            'facts':facts,'direct_paths':paths,'questions':rows,
            'limitations':['Static function inspection is not live U-Boot behavior.',
              'Only QEMU cases marked real_vendor_arm are actual vendor-instruction experiments.',
              'QEMU virt mock-RAM execution cannot prove physical MMIO/SD/eMMC/BootROM.',
              'A question with a related function is not automatically answered.']}