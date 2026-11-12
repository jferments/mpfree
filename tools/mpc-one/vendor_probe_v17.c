/* V17: new execution of genuine MPC One U-Boot branch paths in disposable
 * ARM virt. Uses V16's verified page loader, not a synthetic Fastboot model.
 * All command/storage/boot handoffs intercepted by safe guest trampolines.
 * Never exposes host USB or eMMC to the virtual machine.
 */
#define main mpfree_v16_legacy_main
#include "vendor_probe_v16.c"
#undef main
#ifdef MPFREE_V18_WRAPPING
#define main mpfree_v17_legacy_main
#endif
#define TRAMP ((uintptr_t)0x00900000u)
#define COUNTER ((uintptr_t)0x0060c000u)
static unsigned slots;
static void write16(uintptr_t at,uint16_t v){memcpy((void*)at,&v,2);}
static void write32(uintptr_t at,uint32_t v){memcpy((void*)at,&v,4);}
static int prepare(const char *image){
    if(!init(image)||!map_at(TRAMP,4096))return 0;
    memset((void*)COUNTER,0,512);slots=0;return 1;
}
/* LDR.W PC,[PC,#offset] to a Thumb return stub with counter. The branch
 * requires only eight/ten bytes at the original function entry, avoiding
 * modification of adjacent functions. All changes are guest RAM only. */
static int intercept(uintptr_t at,unsigned ret,unsigned *slot){
    if(slots>=48)return 0;
    unsigned s=slots++;
    uintptr_t tramp=TRAMP+64*s, counter=COUNTER+4*s;
    const uint16_t ins[]={0x4903,0x680a,0x3201,0x600a,
                          (uint16_t)(0x2000u|(ret&255u)),0x4770,0xbf00,0xbf00};
    memcpy((void*)tramp,ins,sizeof(ins));write32(tramp+16,(uint32_t)counter);
    uintptr_t pc=(at+4)&~(uintptr_t)3;
    uintptr_t literal=(at+7)&~(uintptr_t)3;
    if(literal<pc||literal-pc>4095)return 0;
    uintptr_t page=at&~(uintptr_t)4095;
    if(mprotect((void*)page,4096,PROT_READ|PROT_WRITE))return 0;
    write16(at,0xf8df);
    write16(at+2,(uint16_t)(0xf000u|(literal-pc)));
    if(literal!=at+4)write16(at+4,0xbf00);
    write32(literal,(uint32_t)(tramp|1u));
    __builtin___clear_cache((char*)page,(char*)(page+4096));
    if(mprotect((void*)page,4096,PROT_READ|PROT_EXEC))return 0;
    *slot=s;return 1;
}
static int seal(void){
    __builtin___clear_cache((char*)TRAMP,(char*)(TRAMP+4096));
    return mprotect((void*)TRAMP,4096,PROT_READ|PROT_EXEC)==0;
}
static uint32_t calls(unsigned s){return get32(COUNTER+4*s);}
static int oem_branch(const char *name){
    const int shell=strstr(name,"shell")!=NULL;
    const int allowed=strstr(name,"unlocked")!=NULL;
    unsigned command=0,storage=0,format=0,ignored=0;
    if(!intercept(0x00110484u,0,&command)||
       !intercept(0x0010b74cu,0,&storage)||
       !intercept(0x0012e950u,0,&format)||
       !intercept(0x00108804u,0,&ignored)||
       !intercept(0x0012dcfau,0,&ignored)||
       !intercept(0x0010ccb8u,0,&ignored)||
       !intercept(0x00108560u,0,&ignored)||
       !intercept(0x0010d588u,0,&ignored)||
       !intercept(0x0012e96au,0,&ignored)||!seal())return 4;
    const char *word=real_oem_word(shell?0x00122918u:0x00122930u,shell?6:8);
    if(!word)return result(name,false,"vendor-command-pointer-bad");
    set_command(word,shell?6:8);
    volatile uint32_t *flag=(volatile uint32_t*)FLAG;
    *flag=(uint32_t)allowed;oem(HEAP+0x100);
    bool pass=(*flag==(uint32_t)allowed &&
               calls(command)==(unsigned)(shell&&allowed) &&
               calls(storage)==(unsigned)(!shell&&allowed));
    printf("{\"trace\":\"oem-branch\",\"case\":\"%s\",\"command_mock_calls\":%u,"
           "\"storage_mock_calls\":%u,\"format_calls\":%u,\"permission\":%u}\n",
           name,calls(command),calls(storage),calls(format),*flag);
    return result(name,pass,"real-oem-flow-command-or-storage-callback-intercepted");
}
static int boot_done(const char *name){
    unsigned loader=0,format=0,bootm=0,reset=0;
    if(!intercept(0x0010d14cu,0,&loader)||
       !intercept(0x0012e950u,0,&format)||
       !intercept(0x00101744u,0,&bootm)||
       !intercept(0x0011622au,0,&reset)||!seal())return 4;
    ((void(*)(void))0x001226b5u)();
    printf("{\"trace\":\"boot-done\",\"loader\":%u,\"format\":%u,"
           "\"bootm_mock\":%u,\"reset_mock\":%u}\n",
           calls(loader),calls(format),calls(bootm),calls(reset));
    return result(name,calls(loader)==1&&calls(format)==1&&
                       calls(bootm)==1&&calls(reset)==1,
                  "real-boot-done-reached-mocked-bootm-and-reset");
}
static int download_boundary(const char *name){
    unsigned format=0,report=0,callback=0,log=0;
    if(!intercept(0x0012e96au,0,&log)||
       !intercept(0x0012e950u,0,&format)||
       !intercept(0x0012dbf4u,0,&report)||
       !intercept(0x0012247cu,0x20,&callback)||!seal())return 4;
    unsigned value=0;
    if(sscanf(name,"v17_download_%x",&value)!=1)
        return result(name,false,"invalid-hex-download-size");
    char text[64];
    snprintf(text,sizeof(text),"download:%08x",value);
    uintptr_t descriptor=HEAP+0x1000,word=HEAP+0x1200;
    strcpy((char*)word,text);
    memset((void*)descriptor,0,100);put32(descriptor,(uint32_t)word);
    uint32_t sz=get32(0x001229f4u),cnt=get32(0x001229f8u);
    if(!checked_pointer(sz)||!checked_pointer(cnt))
        return result(name,false,"download-pointer-outside-owner-image");
    put32(sz,0xfeedbeefu);put32(cnt,0x12345678u);
    *(volatile uint32_t*)FLAG=1;
    ((void(*)(uintptr_t,uintptr_t))0x00122981u)(HEAP+0x1500,descriptor);
    bool accepted=value>0&&value<=0x70000000u;
    uint32_t observed=get32(sz);
    bool ok=observed==(accepted?value:0u)&&get32(cnt)==0u&&
            calls(callback)==(unsigned)accepted;
    printf("{\"trace\":\"download-boundary\",\"case\":\"%s\",\"requested\":%u,"
           "\"actual\":%u,\"callback\":%u}\n",
           name,value,observed,calls(callback));
    return result(name,ok,"real-download-parser-boundary-with-hardware-mocked");
}
static int receive_canary(const char *name){
    unsigned n=0,declared=0;
    if(sscanf(name,"v17_receive_%u_%u",&n,&declared)!=2||n>8192||declared>8192)
        return result(name,false,"invalid-receive-parameters");
    unsigned fmt=0,log=0,remaining_callback=0;
    if(!intercept(0x0012dbf4u,0,&fmt)||
       !intercept(0x0012e96au,0,&log)||
       !intercept(0x0012247cu,0x20,&remaining_callback)||!seal())return 4;
    uintptr_t source=HEAP+0x2000,context=HEAP+0x400;
    uintptr_t descriptor=HEAP+0x600,vtable=HEAP+0x800;
    uintptr_t output=USB_DATA+0x800;
    uint32_t total=get32(0x00122ad4u),count=get32(0x00122ac4u);
    if(!checked_pointer(total)||!checked_pointer(count))
        return result(name,false,"receiver-pointer-outside-owner-image");
    for(unsigned i=0;i<n;i++)
        ((volatile uint8_t*)source)[i]=(uint8_t)(13u*i+7u);
    memset((void*)output,0xcd,8192+64);
    put32(total,declared);put32(count,0);
    memset((void*)descriptor,0,64);
    put32(context+8,vtable);put32(vtable+16,(uint32_t)(RESPONSE|1u));
    put32(descriptor,source);put32(descriptor+9*4,n);
    receive(context,descriptor);
    unsigned copied=n<declared?n:declared;
    bool identical=memcmp((void*)output,(void*)source,copied)==0;
    bool canary=((volatile uint8_t*)output)[copied]==0xcd;
    bool correct=identical&&canary&&get32(count)==copied&&
        get32(total)==(copied==declared?0u:declared)&&get32(descriptor+9*4)==0;
    printf("{\"trace\":\"receive-copies\",\"case\":\"%s\",\"packet\":%u,"
           "\"declared\":%u,\"count\":%u,\"canary\":%s}\n",
           name,n,declared,get32(count),canary?"true":"false");
    return result(name,correct,"real-receiver-clipping-and-destination-canary");
}
int main(int argc,char **argv){
    if(argc!=3){fputs("usage: vendor_probe_v17 CASE private-verified-p3\n",stderr);return 2;}
    if(strncmp(argv[1],"v17_",4))return mpfree_v16_legacy_main(argc,argv);
    alarm(12);
    if(!prepare(argv[2]))return 3;
    if(get32(0x0012284cu)!=FLAG||get32(0x00122928u)!=FLAG)
        return result(argv[1],false,"auth-global-address-mismatch");
    if(!strncmp(argv[1],"v17_oem_",8))return oem_branch(argv[1]);
    if(!strcmp(argv[1],"v17_boot_completion"))return boot_done(argv[1]);
    if(!strncmp(argv[1],"v17_download_",13))return download_boundary(argv[1]);
    if(!strncmp(argv[1],"v17_receive_",12))return receive_canary(argv[1]);
    return result(argv[1],false,"unknown-v17-case");
}
#ifdef MPFREE_V18_WRAPPING
#undef main
#endif
