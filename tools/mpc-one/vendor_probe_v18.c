/* MPFree V18: new ACTUAL vendor ARM handler coverage on generic ARM virt.
 * Builds on V17's SHA-pinned loader and guest-only callback interception.
 * No physical device, real USB host, MMC access or persistent writes.
 *
 * New experiments: getvar built-in dispatch under locked/unlocked state,
 * Fastboot continue callback, and reboot state handling.
 */
#define MPFREE_V18_WRAPPING 1
#include "vendor_probe_v17.c"
#define GETVAR ((uintptr_t)0x00122d39u)
#define CONTINUE ((uintptr_t)0x00122949u)
#define REBOOT ((uintptr_t)0x00122cf9u)
#define REBOOT_DECISION ((uintptr_t)0x00100edcu)

static int v18_safe_slice(char *dst,size_t cap,uintptr_t ptr){
    if(!checked_pointer(ptr))return 0;
    uintptr_t bound=BASE+LENGTH-ptr;
    size_t max=(size_t)(bound<cap-1?bound:cap-1);
    size_t len=0;
    while(len<max&&((const char*)ptr)[len])len++;
    if(!len||len==max)return 0;
    memcpy(dst,(const void*)ptr,len);
    dst[len]=0;
    return 1;
}
static int v18_state(const char *name,bool isboot){
    uintptr_t cell=isboot?0x00122974u:0x00122958u;
    uintptr_t value=isboot?0x00122978u:0x0012295cu;
    uint32_t global=get32(cell);
    if(!checked_pointer(global))return result(name,false,"state-global-pointer-invalid");
    uintptr_t outer=HEAP+0x300u,inner=HEAP+0x400u;
    put32(global,(uint32_t)outer);
    put32(outer+0x50u,(uint32_t)inner);
    put32(inner+0x10u,0xdeadbeefu);
    unsigned responses=0;
    if(!intercept(RESPONSE,0,&responses)||!seal())return 4;
    if(isboot)((void(*)(void))BOOT)();
    else ((void(*)(void))CONTINUE)();
    uint32_t observed=get32(inner+0x10u);
    printf("{\"trace\":\"callback-state\",\"case\":\"%s\","
           "\"value\":%u,\"expected\":%u,\"responses\":%u}\n",
           name,observed,get32(value),calls(responses));
    return result(name,observed==get32(value)&&calls(responses)==1,
                  "actual-vendor-continue-or-boot-callback-state");
}
static int v18_getvar(const char *name){
    unsigned variant=0,auth=0;
    if(sscanf(name,"v18_getvar_%u_%u",&variant,&auth)!=2||variant>=6||auth>1)
        return result(name,false,"invalid-getvar-parameters");
    const uintptr_t literals[]={0x00122e40u,0x00122e48u,0x00122e50u,
                                0x00122e58u,0x00122e5cu,0x00122e68u};
    char key[72]={0};
    uintptr_t ptr=get32(literals[variant]);
    if(!v18_safe_slice(key,sizeof(key),ptr))
        return result(name,false,"builtin-key-pointer-invalid");
    char *text=(char*)(HEAP+0x1800u);
    int n=snprintf(text,160,"getvar:%s",key);
    if(n<0||n>=160)return result(name,false,"builtin-key-too-large");
    uintptr_t desc=HEAP+0x1600u;
    memset((void*)desc,0,80);
    put32(desc,(uint32_t)(uintptr_t)text);
    unsigned responses=0,saveenv=0,runcommand=0,printf_mock=0;
    /* The formatting helpers are native U-Boot code; we intercept ONLY
       potentially hardware-backed probes and explicit persistence routes. */
    if(!intercept(RESPONSE,0,&responses)||
       !intercept(0x0010b74cu,0,&saveenv)||
       !intercept(0x00110484u,0,&runcommand)||
       !intercept(0x00100ef0u,0,&printf_mock)||
       !seal())return 4;
    *(volatile uint32_t*)FLAG=(uint32_t)auth;
    ((void(*)(uintptr_t,uintptr_t))GETVAR)(0,desc);
    bool ok=calls(responses)>=1 && calls(saveenv)==0 &&
            calls(runcommand)==0 && *(volatile uint32_t*)FLAG==auth;
    printf("{\"trace\":\"getvar-dispatch\",\"case\":\"%s\","
           "\"literal\":\"0x%08lx\",\"key_length\":%u,\"responses\":%u,"
           "\"storage\":%u,\"command\":%u,\"authorization\":%u}\n",
           name,(unsigned long)literals[variant],(unsigned)strlen(key),
           calls(responses),calls(saveenv),calls(runcommand),auth);
    return result(name,ok,"actual-vendor-built-in-getvar-dispatch-mock-hardware");
}
static int v18_reboot(const char *name){
    unsigned branch=0;
    if(sscanf(name,"v18_reboot_%u",&branch)!=1||branch>1)
        return result(name,false,"invalid-reboot-branch");
    unsigned response=0,decision=0;
    if(!intercept(RESPONSE,0,&response)||
       !intercept(REBOOT_DECISION,branch,&decision)||!seal())return 4;
    uint32_t outer=get32(0x00122d2cu);
    if(!checked_pointer(outer))return result(name,false,"reboot-controller-global-invalid");
    put32(outer,(uint32_t)(HEAP+0x200u));
    put32(HEAP+0x250u,(uint32_t)(HEAP+0x300u));
    put32(HEAP+0x300u+0x10u,0xdeadbeefu);
    uintptr_t key=get32(0x00122d24u);
    char token[72]={0};
    if(!v18_safe_slice(token,sizeof(token),key))
        return result(name,false,"reboot-command-pointer-invalid");
    char *text=(char*)(HEAP+0x1400u);
    snprintf(text,160,"%s",token);
    uintptr_t desc=HEAP+0x1200u;
    put32(desc,(uint32_t)(uintptr_t)text);
    ((void(*)(uintptr_t,uintptr_t))REBOOT)(0,desc);
    printf("{\"trace\":\"reboot-selector\",\"case\":\"%s\","
           "\"decision_calls\":%u,\"responses\":%u}\n",
           name,calls(decision),calls(response));
    return result(name,calls(response)==1&&calls(decision)<=1,
                  "actual-vendor-reboot-selector-with-synthetic-reset-decision");
}
int main(int argc,char **argv){
    if(argc!=3){fprintf(stderr,"usage: probe CASE private-P3-image\n");return 2;}
    if(strncmp(argv[1],"v18_",4))return mpfree_v17_legacy_main(argc,argv);
    alarm(10);
    if(!prepare(argv[2]))return 3;
    if(get32(0x0012284cu)!=FLAG||get32(0x00122928u)!=FLAG)
        return result(argv[1],false,"mismatched-pinned-auth-global");
    if(!strcmp(argv[1],"v18_continue_state"))return v18_state(argv[1],false);
    if(!strcmp(argv[1],"v18_boot_state"))return v18_state(argv[1],true);
    if(!strncmp(argv[1],"v18_getvar_",11))return v18_getvar(argv[1]);
    if(!strncmp(argv[1],"v18_reboot_",11))return v18_reboot(argv[1]);
    return result(argv[1],false,"unknown-v18-case");
}
