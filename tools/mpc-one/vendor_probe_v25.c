/* MPFree V25 -- execute real native U-Boot "mmc" command HANDLER
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
#define MPFREE_V25_WRAPPING 1
#include "vendor_probe_v24.c"
#define V25_MMC_HANDLER ((uintptr_t)0x00104f11u)
#define V25_MMC_DESCRIPTOR ((uintptr_t)0x00144ca4u)
#define V25_ARGV (HEAP+0x8000u)
#define V25_STRINGS (HEAP+0x8800u)
static const char *const v25_argv[20][6]={
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
static int v25_guest_mmc(const char *name){
    unsigned which=0;
    if(sscanf(name,"v25_native_mmc_%u",&which)!=1 || which>=20u)
        return result(name,false,"unexpected-V25-MMC-case");
    if(!v24_original_string(get32(V25_MMC_DESCRIPTOR),"mmc"))
        return result(name,false,"original-native-MMC-descriptor-name-mismatch");
    if(get32(V25_MMC_DESCRIPTOR+12u)!=V25_MMC_HANDLER)
        return result(name,false,"original-MMC-command-handler-pointer-mismatch");
    if(!v24_setup_mmc_registry())
        return result(name,false,"failed-safe-synthetic-MMC-registry");
    uint32_t original_permission=get32(FLAG);
    uint32_t original_handler=get32(V25_MMC_DESCRIPTOR+12u);
    unsigned init=0,transport=0,write=0,erase=0,persist=0,reset=0,log=0;
    if(!intercept(0x0011daf4u,0u,&init) ||
       !intercept(0x0011ca78u,1u,&transport) ||
       !intercept(0x0011de50u,0u,&write) ||
       !intercept(0x0011dd4cu,0u,&erase) ||
       !intercept(0x00108110u,1u,&persist) ||
       !intercept(0x0011622au,1u,&reset) ||
       !intercept(0x0012e96au,0u,&log))
        return 4;
    memset((void*)V25_ARGV,0,6u*sizeof(uint32_t));
    memset((void*)V25_STRINGS,0,0x600);
    uintptr_t cursor=V25_STRINGS;
    unsigned argc=0;
    for(unsigned i=0;i<6 && v25_argv[which][i];i++){
        const char *text=v25_argv[which][i];
        size_t n=strlen(text)+1;
        if(cursor+n>V25_STRINGS+0x600)
            return result(name,false,"arg-overflow");
        memcpy((void*)cursor,text,n);
        put32(V25_ARGV+i*sizeof(uint32_t),(uint32_t)cursor);
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
        (int(*)(uint32_t,uint32_t,uint32_t,uint32_t))V25_MMC_HANDLER;
    v21_phase=41;
    int status=real_native_mmc((uint32_t)V25_MMC_DESCRIPTOR,
                               0u,argc,(uint32_t)V25_ARGV);
    v21_phase=42;
    bool guard=(calls(write)==0u && calls(erase)==0u &&
                calls(persist)==0u && calls(reset)==0u &&
                get32(FLAG)==original_permission &&
                get32(V25_MMC_DESCRIPTOR+12u)==original_handler);
    /* A completed query is evidence that the original command handler
       ran in fake-memory isolation. It NEVER proves a physical MMC
       device number or a usable SD boot sequence. */
    printf("{\"trace\":\"v25-original-native-mmc-command-handler\","
           "\"test\":\"%s\",\"argv0\":\"%s\",\"argv1\":\"%s\","
           "\"argv2\":\"%s\",\"argc\":%u,"
           "\"owner_handler_return\":%d,\"mock_device_init_calls\":%u,"
           "\"blocked_controller_transport_calls\":%u,"
           "\"blocked_real_write_callbacks\":%u,"
           "\"blocked_real_erase_callbacks\":%u,"
           "\"blocked_saveenv_calls\":%u,"
           "\"blocked_reset_calls\":%u,"
           "\"low_level_MMC_MMIO_executed\":false,"
           "\"synthetic_backend_only\":true,"
           "\"original_handler_returned\":true}\n",
           name,v25_argv[which][0],
           v25_argv[which][1]?v25_argv[which][1]:"",
           v25_argv[which][2]?v25_argv[which][2]:"",
           argc,status,calls(init),calls(transport),calls(write),
           calls(erase),calls(persist),calls(reset));
    return result(name,guard,
       "original-native-Uboot-MMC-handler-returned-with-physical-write-guard");
}
int main(int argc,char **argv){
    if(argc!=3){
        fputs("usage: vendor_probe_v25 CASE SHA-pinned-owner-P3.raw\n",stderr);
        return 2;
    }
    if(strncmp(argv[1],"v25_",4u)!=0)
        return mpfree_v24_legacy_main(argc,argv);
    alarm(12);
    if(!v21_hooks(argv[1]) || !prepare(argv[2]))return 3;
    if(get32(0x0012284cu)!=FLAG || get32(0x00122928u)!=FLAG)
        return result(argv[1],false,"original-owner-authorization-cell-not-verified");
    if(!strncmp(argv[1],"v25_native_mmc_",15u))
        return v25_guest_mmc(argv[1]);
    return result(argv[1],false,"unknown-V25-case");
}
