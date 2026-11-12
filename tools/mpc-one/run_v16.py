#!/usr/bin/env python3
"""MPFree V16: evidence-linked 85-question research and ACTUAL vendor ARM probes.

ONE private SHA-indexed ZIP. Concurrent read-only Ghidra P1/P3, verified V14/V15
cross-analysis, and staged QEMU ARM virt experiments against a private SHA-pinned
original vendor P3 image with mock RAM/USB callbacks and no host device access.

Use --offline-only for reproducible static analysis without Ghidra/QEMU.
Never performs physical Fastboot, SSH, update, MMC, firmware, or OTP actions.
"""
from __future__ import annotations
import argparse, hashlib, importlib.util, json, os, pathlib, pty, re, select
import shutil, signal, stat, subprocess, sys, tempfile, time, traceback, zipfile
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
from pathlib import Path

HERE=Path(__file__).resolve().parent
REPO=HERE.parents[1]
HOME=Path.home()
WORK=HOME/'mpfree-work'
SHA_P3='b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a'
SHA_P1='6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f'
EVIDENCE=HERE/'evidence_v16.py'
GH_SCRIPT=REPO/'tools/ghidra/MPFreeV16LiveStatic.py'
C_SOURCE=HERE/'vendor_probe_v16.c'
ALLOWED={'.json','.log','.txt','.md','.csv','.tsv'}

def module(path,name):
    spec=importlib.util.spec_from_file_location(name,path)
    obj=importlib.util.module_from_spec(spec);spec.loader.exec_module(obj)
    return obj

def digest(p):
    h=hashlib.sha256()
    with open(p,'rb') as f:
        for block in iter(lambda:f.read(1024*1024),b''):h.update(block)
    return h.hexdigest()

def write(path,data):
    Path(path).write_text(json.dumps(data,indent=2,sort_keys=True)+'\n',encoding='utf8')

def normal(path):
    p=Path(path).expanduser()
    if (p.is_symlink() or not p.is_file() or
        not stat.S_ISREG(p.lstat().st_mode) or str(p).startswith(('/dev/','/proc/','/sys/'))):
        raise ValueError('Unsafe source file: '+str(p))
    return p

def get_ghidra():
    if os.environ.get('GHIDRA_HOME'):
        p=Path(os.environ['GHIDRA_HOME'])/'support/pyghidraRun'
        if p.is_file() and os.access(p,os.X_OK):return [str(p),'-H']
    if shutil.which('snap'):
        try:
            if subprocess.run(['snap','list','ghidra'],stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL,timeout=15).returncode==0:
                return ['snap','run','ghidra.ghidra','-H']
        except Exception:pass
    return None

def gproject(role):
    folder=WORK/'ghidra/projects'
    explicit=os.environ.get('MPFREE_P1_PROJECT' if role=='p1' else 'MPFREE_P3_PROJECT')
    glob=('MPFree_P1_SRAM_*.gpr' if role=='p1' else 'MPFree_P3_Uboot_*.gpr')
    options=([folder/(explicit+'.gpr')] if explicit else
             sorted(folder.glob(glob),key=lambda p:p.stat().st_mtime,reverse=True))
    if not options:raise RuntimeError('Saved analyzed Ghidra project absent: '+role)
    return normal(options[0]).stem

def ghidra(role,dst,export):
    r={'success':False,'stage':'ghidra_'+role,'verified_owner_source':False,
       'hardware_contact':False}
    t=time.monotonic();console=dst/('v16-'+role+'-ghidra-console.log')
    try:
        launcher=get_ghidra()
        if not launcher:raise RuntimeError('Missing PyGhidra headless launcher')
        proj=gproject(role);report=dst/('v16-'+role+'-live-static.json')
        cmd=launcher+[str(WORK/'ghidra/projects'),proj,'-process',
                      'p1-rk3288-sram-stage.bin' if role=='p1' else 'p3-uimage-payload.bin',
                      '-readOnly','-noanalysis','-scriptPath',str(GH_SCRIPT.parent),
                      '-postScript',GH_SCRIPT.name,role,str(report),str(export) if role=='p3' else 'none',
                      '-log',str(dst/('v16-'+role+'-ghidra-engine.log'))]
        with console.open('w') as f:
            p=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT,timeout=2400,check=False)
        if p.returncode:raise RuntimeError('Ghidra returned '+str(p.returncode))
        data=json.loads(report.read_text())
        if not data.get('hash_verified') or data['owner_sha256']!=(SHA_P3 if role=='p3' else SHA_P1):
            raise RuntimeError('Ghidra source hash verification missing')
        if role=='p3' and (not normal(export) or export.stat().st_size!=363649 or digest(export)!=SHA_P3):
            raise RuntimeError('Private P3 source export failed SHA-256 verification')
        r.update({'success':True,'verified_owner_source':True,'report':report.name,
                  'functions':data['statistics']['functions'],'pcode':data['statistics']['high_pcode_ops'],
                  'branch_sites':data['statistics']['branch_sites']})
    except Exception as exc:
        r['error']=str(exc)
        with console.open('a') as f:f.write('\n'+traceback.format_exc()[-2500:])
    r['elapsed_s']=round(time.monotonic()-t,2)
    return r

def source_analysis(dst,p14,p15):
    t=time.monotonic();r={'success':False,'stage':'static_corpus'}
    try:
        e=module(EVIDENCE,'mpfree_v16_evidence')
        a=e.load_verified_zip(p14);b=e.load_verified_zip(p15)
        result=e.evidence(a,b)
        write(dst/'v16-static-85-questions.json',result)
        r.update({'success':True,'question_count':result['summary']['questions'],
                  'classifications':result['summary']['classifications'],
                  'unique_functions_checked':result['summary']['static_functions_inspected'],
                  'direct_paths_checked':len(result['direct_paths'])})
    except Exception as exc:
        r['error']=str(exc)
        (dst/'v16-static-error.log').write_text(traceback.format_exc())
    r['elapsed_s']=round(time.monotonic()-t,2)
    return r

CASES=(['permission_0','permission_1','permission_2','permission_highbit']+
       ['compare_equal','compare_difference','compare_case','compare_zero_length',
        'compare_short_length','compare_early_nul']+
       ['oem_exact','oem_typo','oem_short','oem_suffix','oem_unrelated',
        'oem_shell_denied','oem_env_denied']+
       ['oem_mutate_%02d'%i for i in range(37)]+
       ['oem_trunc_%02d'%i for i in range(38)]+
       ['classify_legacy','classify_fdt','classify_invalid',
        'boot_callback','usb_exit_state']+
       ['receive_one','receive_31','receive_256','receive_4096','receive_clip'])

def qemu_args(fixture):
    q=module(REPO/'tools/mpc-one/arm_qemu_lab.py','mpfree_v16_arm_qemu')
    integ=module(REPO/'tools/mpc-one/run_bootchain_integrated_lab.py',
                 'mpfree_v16_drive_map')
    cmd=q.qemu_args(q.KERNEL)
    cmd.insert(1,'-snapshot')
    cmd=integ.qemu_storage_for_synthetic_model(cmd,fixture)
    if cmd[cmd.index('-nic')+1]!='none' or cmd[cmd.index('-monitor')+1]!='none':
        raise ValueError('Missing QEMU device restrictions')
    if any('usb-host' in x or 'hostdev' in x or 'tap' in x for x in cmd):
        raise ValueError('Host passthrough disallowed')
    return cmd,q

def qemu_real_code(dst,export,tmp,ghresult,args):
    t=time.monotonic()
    r={'success':False,'stage':'real_vendor_qemu','board':'generic ARM virt; NOT RK3288',
       'physical_devices':False,'network':False,'vendor_update_executed':False,
       'vendor_functions_tested':['OEM','string_compare','permission','image_classifier',
                                  'boot_callback','usb_exit_state','receive'],
       'tests_planned':len(CASES),'cases':[]}
    serial=dst/'v16-qemu-vendor-function-console.log'
    proc=None;master=slave=None
    try:
        if not ghresult.get('success'):raise RuntimeError('P3 Ghidra verification failed')
        normal(export)
        if digest(export)!=SHA_P3:raise RuntimeError('Private P3 image does not match pinned owner source')
        for name in ('arm-linux-gnueabihf-gcc','qemu-system-arm','qemu-img','mkfs.ext4'):
            if not shutil.which(name):raise RuntimeError('Missing VM tool '+name)
        directory=tmp/'stage';directory.mkdir(mode=0o700)
        binary=directory/'v16_vendor_probe'
        c=subprocess.run(['arm-linux-gnueabihf-gcc','-std=c11','-O2','-static','-s',
                          '-Wall','-Wextra','-Werror','-o',str(binary),str(C_SOURCE)],
                          capture_output=True,text=True,timeout=120)
        if c.returncode:raise RuntimeError('ARM C compiler failed: '+c.stderr[:1500])
        shutil.copyfile(export,directory/'p3.raw')
        fixture=tmp/'private-arm-v16.ext4'
        with fixture.open('wb') as f:f.truncate(32*1024*1024)
        mk=subprocess.run(['mkfs.ext4','-F','-q','-d',str(directory),str(fixture)],
                           capture_output=True,text=True,timeout=90)
        if mk.returncode:raise RuntimeError('QEMU fixture creation failed: '+mk.stderr[:1200])
        cmd,q=qemu_args(fixture)
        r['guest_kernel_sha256']=digest(q.KERNEL)
        r['guest_clone_sha256']=digest(q.BASE)
        r['verified_image_sha256']=digest(export)
        master,slave=pty.openpty()
        proc=subprocess.Popen(cmd,stdin=slave,stdout=slave,stderr=slave,
                              close_fds=True,start_new_session=True)
        os.close(slave);slave=None
        stamp=str(os.getpid())+'_'+str(int(time.time()))
        lines=['mount -t proc proc /proc 2>/dev/null || true',
               'mkdir -p /mnt/mpfree-v16',
               'mount -t ext4 -o ro /dev/vda /mnt/mpfree-v16']
        lines += ['/mnt/mpfree-v16/v16_vendor_probe '+test+
                  ' /mnt/mpfree-v16/p3.raw' for test in CASES]
        # Single command outstanding at a time; separate ACK for every case,
        # so a guest SIGSEGV or failed test does not stop later cases.
        index=0;awaiting=None;sent=False;data='';result_data=''
        deadline=time.monotonic()+args.qemu_boot_timeout
        def submit():
            nonlocal awaiting,deadline
            marker='V16_ACK_'+stamp+'_'+str(index)
            command=lines[index]+'\necho '+marker+'\n'
            os.write(master,command.encode())
            awaiting=marker;deadline=time.monotonic()+args.case_timeout
        with serial.open('w',encoding='utf8',errors='replace') as log:
            while True:
                if proc.poll() is not None:
                    r['error']='QEMU guest terminated unexpectedly';break
                if time.monotonic()>deadline:
                    r['error']='QEMU guest timed out at command %d/%d'%(index,len(lines));break
                ready,_,_=select.select([master],[],[],0.25)
                if not ready:continue
                try:chunk=os.read(master,65536)
                except OSError:break
                if not chunk:break
                text=chunk.decode('utf8','replace')
                log.write(text);log.flush()
                data+=text;result_data+=text
                if len(data)>1000000:data=data[-1000000:]
                if not sent and re.search(r'(?:^|[\r\n])(?:/ # |# |~ # )',data[-5000:]):
                    sent=True;submit()
                if sent and awaiting and re.search(r'(?m)^'+re.escape(awaiting)+r'[ \t]*$',data.replace('\r','')):
                    index+=1
                    if index==len(lines):break
                    submit()
        seen={}
        for text in result_data.replace('\r','').splitlines():
            if not text.startswith('{"test":'):continue
            try:
                obj=json.loads(text)
                if obj.get('test') in CASES and obj.get('real_vendor_arm') is True:
                    seen[obj['test']]=obj
            except (ValueError,TypeError):pass
        r['cases']=[seen.get(c,{'test':c,'pass':False,'error':'case not observed'}) for c in CASES]
        r['tests_observed']=len(seen)
        r['passed']=sum(1 for x in r['cases'] if x.get('pass') is True)
        r['success']=not r.get('error') and r['tests_observed']==len(CASES) and r['passed']==len(CASES)
        if not r['success'] and 'error' not in r:
            r['error']='One or more actual-vendor function cases failed; see case results and serial log'
    except Exception as exc:
        r['error']=repr(exc)
        with serial.open('a') as log:log.write('\n'+traceback.format_exc()[-2000:])
    finally:
        if proc is not None and proc.poll() is None:
            try:os.killpg(proc.pid,signal.SIGTERM);proc.wait(timeout=10)
            except Exception:
                try:os.killpg(proc.pid,signal.SIGKILL)
                except ProcessLookupError:pass
                proc.wait(timeout=10)
        if master is not None:os.close(master)
        if slave is not None:os.close(slave)
        r['elapsed_s']=round(time.monotonic()-t,2)
        write(dst/'v16-actual-vendor-QEMU-results.json',r)
    return r

def synthesize(dst,prior14,prior15,stages):
    e=module(EVIDENCE,'mpfree_v16_evidence_final')
    a=e.load_verified_zip(prior14);b=e.load_verified_zip(prior15)
    vm={}
    if (dst/'v16-actual-vendor-QEMU-results.json').is_file():
        vm=json.loads((dst/'v16-actual-vendor-QEMU-results.json').read_text())
    report=e.evidence(a,b,vm)
    write(dst/'V16-85-QUESTION-EVIDENCE.json',report)
    text=['# MPFree V16 — actual research results','',
          '**V14/V15 static evidence was SHA-verified and reused, not rerun as new work.**',
          '**The only new vendor execution is in a disposable isolated ARM QEMU guest.**','',
          '## New stage results']
    for name,result in stages.items():
        text.append('- %s: %s, %s seconds, %s'%(name,
            'PASS' if result.get('success') else 'INCOMPLETE',result.get('elapsed_s','?'),
            result.get('error','no error reported')))
    text += ['','## Real vendor ARM function tests',
             '- Cases planned: %d'%len(CASES),
             '- Cases observed: %s'%vm.get('tests_observed',0),
             '- Cases passed: %s'%vm.get('passed',0),
             '- Code families: OEM handler; comparator; permission checker; classifier; boot callback; USB exit; receiver.',
             '- Vendor functions are run ONLY in mock RAM with vendor USB output and peripheral callbacks replaced.',
             '- A case passed in generic ARM virt does NOT prove physical MPC hardware behavior.',
             '', '## 85-question evidence accounting']
    for status,count in sorted(report['summary']['classifications'].items()):
        text.append('- %s: %d'%(status,count))
    text += ['','## Open hardware-specific boundaries',
       '- Real BootROM USB enumeration, eFuse/SPL acceptance, independent eMMC recovery.',
       '- SD media mapping/selection and image relocation on actual MPC One.',
       '- Real Fastboot side effects and persistent authorizations.',
       '- Precise DDR/framebuffer/DTB reservations and a working alternative kernel boot.',
       '', 'All vendor image bytes are PRIVATE and excluded from this ZIP.']
    (dst/'V16-READ-ME-FIRST.md').write_text('\n'.join(text)+'\n')
    return report

def package(dst):
    rows=[];size=0
    for p in sorted(dst.rglob('*')):
        if p.is_symlink():raise ValueError('Symlink in output: '+str(p))
        if p.is_dir():continue
        if not stat.S_ISREG(p.lstat().st_mode) or p.suffix not in ALLOWED:
            raise ValueError('Unexpected output: '+str(p))
        n=p.stat().st_size;size+=n
        if n>32*1024*1024 or size>180*1024*1024:
            raise ValueError('Oversized private research reports')
        rows.append({'name':p.relative_to(dst).as_posix(),'bytes':n,'sha256':digest(p)})
    path=dst.with_suffix('.zip')
    if path.exists():raise FileExistsError('Never overwrite private ZIP')
    with zipfile.ZipFile(path,'x',compression=zipfile.ZIP_DEFLATED,compresslevel=6) as z:
        z.writestr('ZIP_INDEX.json',json.dumps({'schema':'mpfree-private-v16-zip-v1','private':True,'files':rows},indent=2)+'\n')
        for r in rows:z.write(dst/r['name'],arcname=r['name'])
    with zipfile.ZipFile(path) as z:
        if z.testzip() or set(z.namelist())!=set(r['name'] for r in rows)|{'ZIP_INDEX.json'}:
            raise ValueError('ZIP verification failed')
    path.with_suffix('.zip.sha256').write_text(digest(path)+'  '+path.name+'\n')
    return path,len(rows)

def main():
    global WORK
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prior-v14',type=Path)
    parser.add_argument('--prior-v15',type=Path)
    parser.add_argument('--offline-only',action='store_true',
                        help='Run genuine 85-question static evidence audit using only prior ZIPs')
    parser.add_argument('--qemu-boot-timeout',type=int,default=240)
    parser.add_argument('--case-timeout',type=int,default=15)
    parser.add_argument('--work-dir',type=Path,default=WORK)
    args=parser.parse_args()
    if args.qemu_boot_timeout<30 or args.case_timeout<6:parser.error('Guest timeouts too short')
    WORK=args.work_dir.expanduser()
    WORK.mkdir(parents=True,exist_ok=True)
    def latest(version):
        found=sorted(WORK.glob('bootchain-wide-*-'+version+'-*.zip'),
                     key=lambda x:x.stat().st_mtime,reverse=True)
        return found[0] if found else None
    a=normal(args.prior_v14 or latest('v14'))
    b=normal(args.prior_v15 or latest('v15'))
    e=module(EVIDENCE,'mpfree_v16_preflight')
    e.verified_sources(e.load_verified_zip(a),e.load_verified_zip(b))
    os.umask(0o077)
    dst=WORK/('bootchain-wide-'+datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')+'-v16-'+str(os.getpid()))
    dst.mkdir(mode=0o700)
    stages={};started=time.monotonic()
    print('MPFree V16 — owner-private actual vendor instruction experiments',flush=True)
    print('Verified V14:',a,'Verified V15:',b,'Offline only:',args.offline_only,flush=True)
    try:
        if args.offline_only:
            stages['static_corpus']=source_analysis(dst,a,b)
        else:
            with tempfile.TemporaryDirectory(prefix='mpfree-v16-owner-only-',dir=str(WORK)) as tmp_name:
                tmp=Path(tmp_name)
                raw=tmp/'verified-owner-p3.raw'
                with ThreadPoolExecutor(max_workers=4) as pool:
                    pending={
                        'static_corpus':pool.submit(source_analysis,dst,a,b),
                        'ghidra_p1':pool.submit(ghidra,'p1',dst,raw),
                        'ghidra_p3':pool.submit(ghidra,'p3',dst,raw),
                    }
                    # QEMU consumes the verified P3 temporary export as soon as ready;
                    # P1 and static question analysis continue concurrently.
                    p3=pending['ghidra_p3'].result()
                    stages['ghidra_p3']=p3
                    pending['real_vendor_qemu']=pool.submit(qemu_real_code,dst,raw,tmp,p3,args)
                    for name,future in pending.items():
                        if name=='ghidra_p3':continue
                        stages[name]=future.result()
                        print('Stage',name,'PASS' if stages[name].get('success') else 'INCOMPLETE',
                              'elapsed',stages[name].get('elapsed_s'),flush=True)
        synthesize(dst,a,b,stages)
    except KeyboardInterrupt:
        stages['interrupted']={'success':False,'error':'Owner interrupted'}
    except Exception:
        (dst/'v16-unexpected-error.log').write_text(traceback.format_exc())
        stages['unexpected']={'success':False,'error':'See private traceback'}
    finally:
        write(dst/'V16-STAGE-MANIFEST.json',{
            'schema':'mpfree-v16-measured-experiments','utc':datetime.now(timezone.utc).isoformat(),
            'duration_s':round(time.monotonic()-started,2),'stages':stages,
            'prior_v14_sha256':digest(a),'prior_v15_sha256':digest(b),
            'original_source_modified':False,'physical_device_access':False,
            'qemu_host_usb_passthrough':False,'private_vendor_bytes_in_archive':False})
        archive,count=package(dst)
        print('\nSINGLE PRIVATE ZIP:',archive,'\nSHA256:',digest(archive),
              '\nFiles:',count,'\nUpload only this ZIP.',flush=True)

if __name__=='__main__':main()