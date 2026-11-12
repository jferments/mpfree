/* MPFree V19: SHA-pinned vendor ARM execution of recovery selector and
 * environment-backed Fastboot getvar in disposable QEMU ARM virt.
 *
 * The real OEM SPL/U-Boot image is never executed on hardware. This guest
 * executes selected original P3 functions only; all MMIO/ADC and potential
 * persistent callbacks are replaced in PRIVATE GUEST MEMORY by count stubs.
 * No host USB, MMC, eMMC, physical board, fuses or persistent firmware writes.
 */
#define MPFREE_V18_WRAPPING 1
#include "vendor_probe_v17.c"

#define V19_RECOVERY_SELECTOR ((uintptr_t)0x00100fe5u)
#define V19_GETVAR_HANDLER   ((uintptr_t)0x00122d39u)
#define V19_ENV_GETTER       ((uintptr_t)0x00108804u)
#define V19_MODE_PTR_CELL    ((uintptr_t)0x0010114cu)
#define V19_MAGIC_CELL       ((uintptr_t)0x00101150u)
#define V19_FLAG_PTR_CELL    ((uintptr_t)0x00101154u)
#define V19_ADC_PTR_CELL     ((uintptr_t)0x0010113cu)

static int patch_pointer_cell(uintptr_t at,uint32_t value) {
    if(at<BASE || at>=BASE+LENGTH-4)return 0;
    uintptr_t page=at&~(uintptr_t)4095;
    if(mprotect((void*)page,4096,PROT_READ|PROT_WRITE))return 0;
    write32(at,value);
    __builtin___clear_cache((char*)page,(char*)(page+4096));
    return mprotect((void*)page,4096,PROT_READ|PROT_EXEC)==0;
}
static int v19_read_vendor_string(char *dst,size_t n,uintptr_t literal) {
    if(n==0)return 0;
    uintptr_t p=get32(literal);
    if(!checked_pointer(p))return 0;
    size_t limit=(size_t)(BASE+LENGTH-p);
    if(limit>n)limit=n;
    size_t i;
    for(i=0;i<limit;i++){
        char c=((const char*)p)[i];
        if(c==0)break;
        if(c<32||c>126)return 0;
        dst[i]=c;
    }
    if(i==0||i==limit)return 0;
    dst[i]=0;
    return 1;
}
/* Existing V17 intercepted trampoline at TRAMP+64*slot returns an 8-bit
 * immediate in r0. Replace that single return-value instruction with a
 * Thumb literal load: ldr r0,[pc,#12] at +8, using a synthetic pointer at +24.
 * Counting instructions and the BX LR remain intact. */
static int v19_pointer_return(unsigned slot,uintptr_t fake_ptr){
    if(slot>=slots)return 0;
    uintptr_t at=TRAMP+(uintptr_t)64*slot;
    write16(at+8,0x4803);
    write32(at+24,(uint32_t)fake_ptr);
    return 1;
}
static int v19_getvar(const char *name){
    unsigned authorization=0,env_mode=0,variant=0;
    if(sscanf(name,"v19_getvar_%u_%u_%u",
              &variant,&authorization,&env_mode)!=3 ||
       (variant!=4&&variant!=5) || authorization>1 || env_mode>2)
        return result(name,false,"invalid-getvar-case");
    const uintptr_t cell=(variant==4)?0x00122e5cu:0x00122e68u;
    char key[64];
    if(!v19_read_vendor_string(key,sizeof(key),cell))
        return result(name,false,"unrecognized-vendor-key-literal");

    uintptr_t text=HEAP+0x1800u,desc=HEAP+0x1600u;
    snprintf((char*)text,128,"getvar:%s",key);
    memset((void*)desc,0,80);write32(desc,(uint32_t)text);

    unsigned response=0,env=0,persist=0,command=0;
    if(!intercept(RESPONSE,0,&response) ||
       !intercept(V19_ENV_GETTER,0,&env) ||
       !intercept(0x0010b74cu,0,&persist) ||
       !intercept(0x00110484u,0,&command))
        return 4;

    /* The fifth getvar key calls the real U-Boot environment lookup. In V18
       this unstubbed dependency caused a guest SIGSEGV. We replace ONLY
       that lookup, returning a controlled null/empty/nonempty string. */
    uintptr_t replacement=0;
    if(env_mode!=0){
        replacement=HEAP+0x5800u;
        if(env_mode==1)strcpy((char*)replacement,"");
        else strcpy((char*)replacement,"MPFree-owner-lab-value");
    }
    if(!v19_pointer_return(env,replacement) || !seal())return 4;
    *(volatile uint32_t*)FLAG=authorization;
    ((void(*)(uintptr_t,uintptr_t))V19_GETVAR_HANDLER)(0,desc);
    bool ok=calls(response)==1 && calls(persist)==0 &&
            calls(command)==0 && *(volatile uint32_t*)FLAG==authorization;
    /* Environment lookup should be called for variant 4, not variant 5.
       Distinguish null, empty and nonempty fake environment responses. */
    ok=ok && (calls(env)==(unsigned)(variant==4));
    printf("{\"trace\":\"v19-getvar-env\",\"case\":\"%s\","
           "\"key_index\":%u,\"authorization\":%u,\"fake_env_mode\":%u,"
           "\"getter_calls\":%u,\"response_calls\":%u,"
           "\"persistent_calls\":%u,\"command_calls\":%u}\n",
           name,variant,authorization,env_mode,calls(env),calls(response),
           calls(persist),calls(command));
    return result(name,ok,"real-vendor-getvar-with-isolated-environment");
}
static int v19_recovery(const char *name){
    unsigned mode=0,adc_value=255,badprefix=0;
    bool adc=false;
    if(sscanf(name,"v19_recovery_mode_%u",&mode)==1){
        if(mode>255)return result(name,false,"invalid-boot-mode");
    }else if(sscanf(name,"v19_recovery_adc_%u",&adc_value)==1){
        if(adc_value>255)return result(name,false,"invalid-adc-value");
        adc=true;
    }else if(!strcmp(name,"v19_recovery_badprefix")){
        badprefix=1;
    }else return result(name,false,"unrecognized-recovery-case");

    const struct {uintptr_t at;unsigned value;} stubs[]={
       {0x0011b5d0u,1}, /* avoid real environment variable reads */
       {0x0011b6d0u,1}, {0x0011b884u,0},
       {0x001148a4u,0}, {0x0011ebaeu,0},
       {0x0011eadcu,0}, {0x0012e004u,0},
       {0x00115630u,0}, {0x0012e96au,0},
       {0x001147f0u,adc?0u:1u}, /* ADC unavailable unless requested */
       {0x00115ef4u,0}, {0x00115f30u,0}, {0x00115ed8u,0},
       {0x001147d4u,255}, {0x0011a9f8u,0},
       {0x0011aa10u,0},
    };
    unsigned saved[sizeof(stubs)/sizeof(stubs[0])];
    for(size_t i=0;i<sizeof(stubs)/sizeof(stubs[0]);i++){
        if(!intercept(stubs[i].at,stubs[i].value,&saved[i]))
            return 4;
    }
    /* For ADC test, patch only our synthetic return trampoline to write
       the emulated ADC measurement to caller's output buffer (r2) and
       return success. Original vendor caller branch remains intact.
         MOVS r3,#adc_value; STR r3,[r2]; MOVS r0,#0; BX LR
    */
    if(adc){
        uintptr_t trampoline=TRAMP+64u*saved[11];
        write16(trampoline+0,(uint16_t)(0x2300u|adc_value));
        write16(trampoline+2,0x6013u);
        write16(trampoline+4,0x2000u);
        write16(trampoline+6,0x4770u);
    }
    if(!patch_pointer_cell(V19_MODE_PTR_CELL,(uint32_t)(HEAP+0x3100u)) ||
       !patch_pointer_cell(V19_FLAG_PTR_CELL,(uint32_t)(HEAP+0x3110u)) ||
       !patch_pointer_cell(V19_ADC_PTR_CELL,(uint32_t)(HEAP+0x3200u)))
        return 4;
    memset((void*)(HEAP+0x3200u),0,128);
    uint32_t prefix=get32(V19_MAGIC_CELL)&0xffffff00u;
    uint32_t input=prefix|mode;
    if(badprefix)input=(prefix^0x01000000u);
    write32(HEAP+0x3100u,input);
    write32(HEAP+0x3110u,0u);
    if(!seal())return 4;
    int (*run)(void)=(int(*)(void))V19_RECOVERY_SELECTOR;
    int code=run();
    uint32_t observed=get32(HEAP+0x3110u);
    uint32_t wiped=get32(HEAP+0x3100u);
    bool expected=adc ? (adc_value<100) :
            (!badprefix && (mode==1||mode==9));
    bool ok=code==0 && wiped==0 && observed==(uint32_t)expected;
    printf("{\"trace\":\"v19-recovery-policy\",\"case\":\"%s\","
           "\"requested_mode\":%u,\"adc_mode\":%s,\"adc_value\":%u,"
           "\"recovery_observed\":%u,\"recovery_expected\":%u,"
           "\"pmu_register_cleared\":%s,\"return_code\":%d}\n",
           name,mode,adc?"true":"false",adc_value,observed,(unsigned)expected,
           wiped==0?"true":"false",code);
    return result(name,ok,"real-vendor-recovery-policy-with-mock-PMU-and-ADC");
}
int main(int argc,char **argv){
    if(argc!=3){fprintf(stderr,"usage: vendor_probe_v19 CASE private-P3-image\n");return 2;}
    if(strncmp(argv[1],"v19_",4))return mpfree_v17_legacy_main(argc,argv);
    alarm(12);
    if(!prepare(argv[2]))return 3;
    if(get32(0x0012284cu)!=FLAG||get32(0x00122928u)!=FLAG)
        return result(argv[1],false,"verified-authorization-global-mismatch");
    if(!strncmp(argv[1],"v19_getvar_",11))return v19_getvar(argv[1]);
    if(!strncmp(argv[1],"v19_recovery_",13))return v19_recovery(argv[1]);
    return result(argv[1],false,"unknown-v19-case");
}
