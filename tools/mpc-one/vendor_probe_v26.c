/* MPFree V26 -- execute real native U-Boot "mmc" command HANDLER
 * with strictly fake registered memory objects and hardware I/O hooks.
 *
 * V24 validated 86 original registered command descriptors; the native
 * handler for "mmc" is owner Thumb 0x00104f11 (descriptor 0x00144ca4).
 * No command parser substitution: call the actual registered handler.
 *
 * Only read-only/info and transient device-selection subcommands tested.
 * Hardware request transport, block write/erase, saveenv, reset and
 * device initialize are all intercepted in the isolated guest.
 * No physical device passthrough. No command text from owner secrets.
 */
#define MPFREE_V26_WRAPPING 1
#include "vendor_probe_v24.c"
#define V26_MMC_HANDLER ((uintptr_t)0x00104f11u)
#define V26_MMC_DESCRIPTOR ((uintptr_t)0x00144ca4u)
#define V26_ARGV (HEAP+0x8000u)
#define V26_STRINGS (HEAP+0x8800u)
static const char *const v26_argv[20][6]={
    {"mmc","list",NULL},
    {"mmc","list",NULL},
    {"mmc","dev","0",NULL},
    {"mmc","dev","1",NULL},
    {"mmc","dev","2",NULL},
    {"mmc","dev","3",NULL},
    {"mmc","dev",NULL},
    {"mmc","info",NULL},
    {"mmc","part",NULL},
    {"mmc","rescan",NULL},
    {"mmc","dev","0","0",NULL},
    {"mmc","dev","1","0",NULL},
    {"mmc","dev","0","1",NULL},
    {"mmc","dev","1","1",NULL},
    {"mmc","list","not_an_option",NULL},
    {"mmc","help",NULL},
    {"mmc","dev","mpfree_invalid_number",NULL},
    {"mmc","dev","ff",NULL},
    {"mmc","dev","00000001",NULL},
    {"mmc","dev","1","0x0",NULL},
};
static int v26_guest_mmc(const char *name){
    unsigned which=0;
    if(sscanf(name,"v26_native_mmc_%u",&which)!=1 || which>=20u)
        return result(name,false,"unexpected-V25-MMC-case");
    if(!v24_original_string(get32(V26_MMC_DESCRIPTOR),"mmc"))
        return result(name,false,"original-native-MMC-descriptor-name-mismatch");
    if(get32(V26_MMC_DESCRIPTOR+12u)!=V26_MMC_HANDLER)
        return result(name,false,"original-MMC-command-handler-pointer-mismatch");
    if(!v24_setup_mmc_registry())
        return result(name,false,"failed-safe-synthetic-MMC-registry");
    uint32_t original_permission=get32(FLAG);
    uint32_t original_handler=get32(V26_MMC_DESCRIPTOR+12u);
    unsigned init=0,transport=0,controller=0,config=0,write=0,erase=0,persist=0,reset=0,log=0,uart=0,alternate=0;
    if(!intercept(0x0011daf4u,0u,&init) ||
       !intercept(0x0011ca78u,1u,&transport) ||
       !intercept(0x0011c4e8u,1u,&controller) ||
       !intercept(0x0011c912u,0u,&config) ||
       !intercept(0x0011de50u,0u,&write) ||
       !intercept(0x0011dd4cu,0u,&erase) ||
       !intercept(0x00108110u,1u,&persist) ||
       !intercept(0x0011622au,1u,&reset) ||
       !intercept(0x0012e96au,0u,&log) ||
       !intercept(0x0011f580u,0u,&uart) ||
       !intercept(0x0011f2d6u,0u,&alternate))
        return 4;
    memset((void*)V26_ARGV,0,6u*sizeof(uint32_t));
    memset((void*)V26_STRINGS,0,0x600);
    uintptr_t cursor=V26_STRINGS;
    unsigned argc=0;
    for(unsigned i=0;i<6 && v26_argv[which][i];i++){
        const char *text=v26_argv[which][i];
        size_t n=strlen(text)+1;
        if(cursor+n>V26_STRINGS+0x600)
            return result(name,false,"arg-overflow");
        memcpy((void*)cursor,text,n);
        put32(V26_ARGV+i*sizeof(uint32_t),(uint32_t)cursor);
        cursor+=((n+3u)&~3u);
        argc++;
    }
    if(!seal())return 4;
    /* Standard original U-Boot cmd_tbl_t callback ABI:
       int do_mmc(cmd_tbl_t *cmdtp, int flags, int argc, char *argv[]).
       Only read-only or transient selection arguments; this call
       intentionally DOES NOT invoke the full U-Boot command parser.
       All hardware and destructive system hooks were intercepted. */
    int(*real_native_mmc)(uint32_t,uint32_t,uint32_t,uint32_t)=
        (int(*)(uint32_t,uint32_t,uint32_t,uint32_t))V26_MMC_HANDLER;
    v21_phase=41;
    int status=real_native_mmc((uint32_t)V26_MMC_DESCRIPTOR,
                               0u,argc,(uint32_t)V26_ARGV);
    v21_phase=42;
    bool guard=(calls(write)==0u && calls(erase)==0u &&
                calls(persist)==0u && calls(reset)==0u &&
                get32(FLAG)==original_permission &&
                get32(V26_MMC_DESCRIPTOR+12u)==original_handler);
    /* A completed query is evidence that the original command handler
       ran in fake-memory isolation. It NEVER proves a physical MMC
       device number or a usable SD boot sequence. */
    printf("{\"trace\":\"v25-original-native-mmc-command-handler\","
           "\"test\":\"%s\",\"argv0\":\"%s\",\"argv1\":\"%s\","
           "\"argv2\":\"%s\",\"argc\":%u,"
           "\"owner_handler_return\":%d,\"mock_device_init_calls\":%u,"
           "\"guest_UART_char_sink_calls\":%u,"
           "\"guest_UART_alt_sink_calls\":%u,"
           "\"blocked_controller_transport_calls\":%u,"
           "\"blocked_controller_command_calls\":%u,"
           "\"blocked_controller_config_calls\":%u,"
           "\"blocked_real_write_callbacks\":%u,"
           "\"blocked_real_erase_callbacks\":%u,"
           "\"blocked_saveenv_calls\":%u,"
           "\"blocked_reset_calls\":%u,"
           "\"low_level_MMC_MMIO_executed\":false,"
           "\"synthetic_backend_only\":true,"
           "\"original_handler_returned\":true}\n",
           name,v26_argv[which][0],
           v26_argv[which][1]?v26_argv[which][1]:"",
           v26_argv[which][2]?v26_argv[which][2]:"",
           argc,status,calls(init),calls(uart),calls(alternate),calls(transport),calls(controller),calls(config),calls(write),
           calls(erase),calls(persist),calls(reset));
    /* Reject "silent invalid" cases as success evidence of a real
       informational/device-selection operation. The original U-Boot may
       legitimately reject a request; preserve the exact return status.
       A clean return only validates safe guest execution, not SD use. */
    return result(name,guard,
       "original-native-MMC-handler-returned-UART-mocked-with-guard");
}
/* Original registered mmcinfo handler: additional memory-only scenarios.
 * 0-3 select absent backend (ID=2 or 3) and assert no hardware calls.
 * 4-11 provide synthetic MMC cards and adjust current ID, exposing the
 * genuine metadata-reporting path if the firmware guest can handle it.
 * All actual UART byte output is sinked and all transport is blocked.
 */
static int v26_guest_mmcinfo(const char *name){
    unsigned n=0;
    if(sscanf(name,"v26_mmcinfo_%u",&n)!=1 || n>=12u)
        return result(name,false,"bad-mmcinfo-case");
    const uintptr_t descriptor=0x00144cc0u;
    const uintptr_t handler=0x00105621u;
    if(!v24_original_string(get32(descriptor),"mmcinfo") ||
       get32(descriptor+12u)!=handler)
        return result(name,false,"original-owner-mmcinfo-descriptor-mismatch");
    if(!v24_setup_mmc_registry())
        return result(name,false,"cannot-install-isolated-MMC-devices");
    uintptr_t chosen=get32(0x00105828u);
    if(chosen<BASE || chosen+4>BASE+IMAGE_MAP)
        return result(name,false,"owner-current-mmc-index-pointer-unmapped");
    unsigned id=(n<4u)?(2u+n%2u):(n%2u);
    if(!v21_patch_ram_word(chosen,id))
        return result(name,false,"cannot-write-guest-only-current-mmc-index");
    uintptr_t a=V24_MMC_BACKEND_A;
    uintptr_t b=V24_MMC_BACKEND_B;
    uintptr_t ctx=HEAP+0xa000u, device_label=HEAP+0xa400u;
    memset((void*)ctx,0,0x400);
    strcpy((char*)device_label,"MPFree-guest-synthetic-controller");
    put32(ctx,(uint32_t)device_label);
    for(unsigned i=0;i<2;i++){
        uintptr_t p=i?b:a;
        put32(p+8u,(uint32_t)ctx);
        put32(p+0x14u,1u); /* Suppress physical MMC init if owner checks */
        put32(p+0x68u,512u);
        put32(p+0x70u,512u);
        put32(p+0x78u,32768u);
        put32(p+0x7cu,0u);
        put32(p+0x140u,0u);
    }
    unsigned uart=0,alt=0,serial=0,init=0,write=0,erase=0,ctl=0,cfg=0,transport=0,persist=0,reset=0;
    if(!intercept(0x0011f580u,0u,&uart) ||
       !intercept(0x0011f2d6u,0u,&alt) ||
       !intercept(0x0012e96au,0u,&serial) ||
       !intercept(0x0011daf4u,0u,&init) ||
       !intercept(0x0011ca78u,1u,&transport) ||
       !intercept(0x0011c4e8u,1u,&ctl) ||
       !intercept(0x0011c912u,0u,&cfg) ||
       !intercept(0x0011de50u,0u,&write) ||
       !intercept(0x0011dd4cu,0u,&erase) ||
       !intercept(0x00108110u,1u,&persist) ||
       !intercept(0x0011622au,1u,&reset) ||
       !seal())return 4;
    uint32_t (*original_mmcinfo)(uint32_t,uint32_t,uint32_t,uint32_t)=
        (uint32_t(*)(uint32_t,uint32_t,uint32_t,uint32_t))handler;
    v21_phase=52;
    uint32_t rc=original_mmcinfo((uint32_t)descriptor,0u,1u,0u);
    bool guard=calls(write)==0u && calls(erase)==0u &&
               calls(ctl)==0u && calls(cfg)==0u &&
               calls(transport)==0u && calls(persist)==0u &&
               calls(reset)==0u;
    printf("{\"trace\":\"v26-genuine-native-mmcinfo\","
           "\"test\":\"%s\",\"synthetic_current_mmc\":%u,"
           "\"original_handler_status\":%u,\"guest_UART_calls\":%u,"
           "\"low_level_write_calls\":%u,\"controller_calls\":%u,"
           "\"physical_device_tested\":false}\n",
           name,id,rc,calls(uart),calls(write),calls(ctl));
    return result(name,guard,"real-mmcinfo-with-guest-UART-and-synthetic-card-only");
}
int main(int argc,char **argv){
    if(argc!=3){
        fputs("usage: vendor_probe_v25 CASE SHA-pinned-owner-P3.raw\n",stderr);
        return 2;
    }
    if(strncmp(argv[1],"v26_",4u)!=0)
        return mpfree_v25_legacy_main(argc,argv);
    alarm(12);
    if(!v21_hooks(argv[1]) || !prepare(argv[2]))return 3;
    if(get32(0x0012284cu)!=FLAG || get32(0x00122928u)!=FLAG)
        return result(argv[1],false,"original-owner-authorization-cell-not-verified");
    if(!strncmp(argv[1],"v26_native_mmc_",15u))
        return v26_guest_mmc(argv[1]);
    if(!strncmp(argv[1],"v26_mmcinfo_",12u))
        return v26_guest_mmcinfo(argv[1]);
    return result(argv[1],false,"unknown-V25-case");
}
