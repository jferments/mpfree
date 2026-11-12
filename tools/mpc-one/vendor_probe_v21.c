/* MPFree V21: backend registry and indirect-transfer diagnostics.
 *
 * Executes only verified owner-P3 U-Boot functions in a disposable ARM-virt
 * guest. No physical USB/MMC/eMMC, no real storage callback, no persistent
 * writes. Captures ARM fault PC/LR/R5/R8 on V20's previously unresolved
 * Illegal-instruction paths. Guest process isolation remains per case.
 */
#define MPFREE_V21_WRAPPING 1
#include "vendor_probe_v20.c"
#include <signal.h>
#include <ucontext.h>

#define V21_LOOKUP ((uintptr_t)0x0011c3adu)
#define V21_PROJECT ((uintptr_t)0x0011c3edu)
#define V21_SENTINEL_CELL ((uintptr_t)0x0011c3d4u)
#define V21_BACKEND_A (HEAP+0x3000u)
#define V21_BACKEND_B (HEAP+0x3800u)
#define V21_BACKEND_C (HEAP+0x4000u)

static volatile sig_atomic_t v21_phase=0;
static const char *v21_case="";
static void v21_fault(int signo,siginfo_t *info,void *ptr){
    (void)info;
    unsigned pc=0,lr=0,r5=0,r8=0,r0=0,cpsr=0;
#if defined(__arm__)
    const ucontext_t *ctx=(const ucontext_t*)ptr;
    if(ctx){
        pc=(unsigned)ctx->uc_mcontext.arm_pc;
        lr=(unsigned)ctx->uc_mcontext.arm_lr;
        r5=(unsigned)ctx->uc_mcontext.arm_r5;
        r8=(unsigned)ctx->uc_mcontext.arm_r8;
        r0=(unsigned)ctx->uc_mcontext.arm_r0;
        cpsr=(unsigned)ctx->uc_mcontext.arm_cpsr;
    }
#else
    (void)ptr;
#endif
    /* This diagnostic is intentionally NOT a passed test. */
    dprintf(STDOUT_FILENO,
        "{\"v21_guest_fault\":\"signal\",\"test\":\"%s\","
        "\"signal\":%d,\"phase\":%d,"
        "\"pc\":\"0x%08x\",\"lr\":\"0x%08x\","
        "\"r5\":\"0x%08x\",\"r8\":\"0x%08x\","
        "\"r0\":\"0x%08x\",\"cpsr\":\"0x%08x\"}\n",
        v21_case,signo,(int)v21_phase,pc,lr,r5,r8,r0,cpsr);
    _exit(128+signo);
}
static int v21_hooks(const char *name){
    v21_case=name;
    struct sigaction sa;
    memset(&sa,0,sizeof(sa));
    sa.sa_sigaction=v21_fault;
    sa.sa_flags=SA_SIGINFO|SA_RESETHAND;
    sigemptyset(&sa.sa_mask);
    if(sigaction(SIGILL,&sa,NULL) || sigaction(SIGSEGV,&sa,NULL) ||
       sigaction(SIGBUS,&sa,NULL))
        return 0;
    return 1;
}
static int v21_patch_ram_word(uintptr_t addr,uint32_t value){
    /* If a verified owner pointer refers to mapped code/data rather than
       the guest-writable BSS page, open only that GUEST page for the write. */
    if(addr<BASE || addr+4>BASE+IMAGE_MAP)return 0;
    uintptr_t page=addr&~(uintptr_t)4095;
    bool bss=(page==0x0014d000u);
    if(!bss && mprotect((void*)page,4096,PROT_READ|PROT_WRITE))
        return 0;
    put32(addr,value);
    if(!bss){
        __builtin___clear_cache((char*)page,(char*)(page+4096));
        return mprotect((void*)page,4096,PROT_READ|PROT_EXEC)==0;
    }
    return 1;
}
static int v21_registry(const char *name){
    unsigned variant=0;
    if(sscanf(name,"v21_registry_%u",&variant)!=1||variant>4)
        return result(name,false,"invalid-registry-probe");
    uintptr_t sentinel=get32(V21_SENTINEL_CELL);
    if(sentinel<BASE || sentinel>=BASE+IMAGE_MAP)
        return result(name,false,"owner-backend-sentinel-outside-mapped-image");
    unsigned logger=0;
    if(!intercept(0x0012e96au,0,&logger) || !seal())
        return 4;
    memset((void*)V21_BACKEND_A,0,0x600);
    memset((void*)V21_BACKEND_B,0,0x600);
    memset((void*)V21_BACKEND_C,0,0x600);
    put32(V21_BACKEND_A,(uint32_t)V21_BACKEND_B);
    put32(V21_BACKEND_B,(uint32_t)sentinel);
    put32(V21_BACKEND_A+0xccu,11u);
    put32(V21_BACKEND_B+0xccu,27u);
    uintptr_t first=variant==3?sentinel:V21_BACKEND_A;
    if(!v21_patch_ram_word(sentinel,(uint32_t)first))
        return result(name,false,"cannot-initialize-guest-list-head");
    unsigned requested=(variant==0)?11u:(variant==1)?27u:
                       (variant==2)?44u:(variant==3)?11u:27u;
    volatile uint32_t(*lookup)(unsigned)=(uint32_t(*)(unsigned))V21_LOOKUP;
    volatile uint32_t observed=lookup(requested);
    uintptr_t expected=(variant==0)?V21_BACKEND_A:
                       (variant==1||variant==4)?V21_BACKEND_B:0u;
    if(variant==3)expected=0u;
    bool ok=(uintptr_t)observed==expected;
    if(variant==4){
        /* Exercise genuine projection on the returned backend object. */
        uint32_t(*project)(uint32_t)=(uint32_t(*)(uint32_t))V21_PROJECT;
        uint32_t nested=project(observed);
        ok=ok&&nested==(uint32_t)(V21_BACKEND_B+0xc8u);
    }
    printf("{\"trace\":\"v21-backend-linked-list\",\"test\":\"%s\","
           "\"sentinel\":\"0x%08lx\",\"requested_id\":%u,"
           "\"observed_backend\":\"0x%08x\",\"expected_backend\":\"0x%08lx\","
           "\"log_calls\":%u,\"real_owner_lookup\":true}\n",
           name,(unsigned long)sentinel,requested,(unsigned)observed,
           (unsigned long)expected,calls(logger));
    return result(name,ok,"real-vendor-linked-list-backend-lookup-isolated");
}
static int v21_transfer(const char *name){
    unsigned variant=0;
    if(sscanf(name,"v21_transfer_%u",&variant)!=1||variant>5)
        return result(name,false,"invalid-transfer-probe");
    unsigned initial=0,lookup=0,ready=0,load=0,extent=0;
    unsigned align=0,callback=0,done=0,logger=0;
    if(!intercept(0x0010b732u,0,&initial) ||
       !intercept(0x0011c3acu,0,&lookup) ||
       !intercept(0x0011daf4u,0,&ready) ||
       !intercept(0x0010cdf4u,0,&load) ||
       !intercept(0x0010b728u,0,&extent) ||
       !intercept(0x0012ee08u,16u,&align) ||
       !intercept(0x0010d14cu,0,&done) ||
       !intercept(0x0012e96au,0,&logger) ||
       !intercept(0x0012ccd0u,variant==1?0u:16u,&callback))
        return 4;
    memset((void*)V21_BACKEND_C,0,0x600);
    put32(V21_BACKEND_C+0x6cu,0x200u);
    uintptr_t callback_pointer=TRAMP+64u*(uintptr_t)callback+1u;
    put32(V21_BACKEND_C+0x130u,(uint32_t)callback_pointer);
    if(!v20_return_pointer(lookup,V21_BACKEND_C) || !seal())
        return 4;

    /* Control: execute the callback stub directly BEFORE the backend code.
       This distinguishes a broken trampoline from a corrupted object or
       a U-Boot indirect-call-site issue. */
    int(*cb)(void)=(int(*)(void))callback_pointer;
    if(variant==2||variant==3){
        v21_phase=1;
        int directly=cb();
        printf("{\"v21_checkpoint\":\"direct-trampoline\","
               "\"test\":\"%s\",\"return\":%d,\"calls\":%u,"
               "\"pointer\":\"0x%08lx\"}\n",
               name,directly,calls(callback),(unsigned long)callback_pointer);
        if(variant==2)
            return result(name, directly==16 && calls(callback)==1,
                          "isolated-guest-direct-vendor-callback-stub-preflight");
    }
    v21_phase=2;
    uint32_t(*project)(uint32_t)=(uint32_t(*)(uint32_t))V21_PROJECT;
    uint32_t projected=project((uint32_t)V21_BACKEND_C);
    uint32_t actual_pointer=get32(projected+0x68u);
    printf("{\"v21_checkpoint\":\"before-owner-wrapper\","
           "\"test\":\"%s\",\"backend\":\"0x%08lx\","
           "\"projected\":\"0x%08x\",\"callback_ptr\":\"0x%08x\","
           "\"expected_callback\":\"0x%08lx\",\"calls_before\":%u}\n",
           name,(unsigned long)V21_BACKEND_C,projected,actual_pointer,
           (unsigned long)callback_pointer,calls(callback));
    fflush(stdout);
    if(actual_pointer!=(uint32_t)callback_pointer)
        return result(name,false,"backend-projection-pointer-not-same");
    v21_phase=3;
    int(*vendor)(void)=(int(*)(void))V20_BACKEND;
    int rc=vendor();
    v21_phase=4;
    unsigned expected_calls=(variant==3)?2u:1u;
    bool ok=(calls(callback)==expected_calls && calls(lookup)==1 &&
             calls(ready)==1 && calls(load)==1 && calls(extent)==1);
    /* Successful callback returning expected transfer length => status 0;
       different return => status 1. */
    int want_rc=(variant==1)?1:0;
    ok=ok&&(rc==want_rc);
    printf("{\"trace\":\"v21-storage-indirect-transfer\","
           "\"test\":\"%s\",\"real_wrapper_return\":%d,"
           "\"want_return\":%d,\"callback_calls\":%u,"
           "\"expected_calls\":%u,\"lookup_calls\":%u,"
           "\"ready_calls\":%u,\"load_calls\":%u,"
           "\"extent_calls\":%u,\"completion_log_calls\":%u}\n",
           name,rc,want_rc,calls(callback),expected_calls,
           calls(lookup),calls(ready),calls(load),calls(extent),calls(done));
    return result(name,ok,"actual-owner-backend-indirect-call-with-fake-media");
}
int main(int argc,char **argv){
    if(argc!=3){fputs("usage: v21 CASE private-pinned-owner-P3\n",stderr);return 2;}
    if(strncmp(argv[1],"v21_",4))return mpfree_v20_legacy_main(argc,argv);
    alarm(12);
    if(!v21_hooks(argv[1]))return 3;
    if(!prepare(argv[2]))return 3;
    if(get32(0x0012284cu)!=FLAG||get32(0x00122928u)!=FLAG)
        return result(argv[1],false,"verified-owner-auth-global-pointer-mismatch");
    if(!strncmp(argv[1],"v21_registry_",13))return v21_registry(argv[1]);
    if(!strncmp(argv[1],"v21_transfer_",13))return v21_transfer(argv[1]);
    return result(argv[1],false,"unrecognized-v21-guest-probe");
}
