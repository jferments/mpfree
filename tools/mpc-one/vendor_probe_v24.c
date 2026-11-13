/* MPFree V24: ORIGINAL U-Boot native command registration and MMC
 * device-ID selection. Command *handlers* are NEVER EXECUTED.
 *
 * Actual functions:
 *   FUN_00110CCC find a command by name (7 uint32_t descriptors)
 *   FUN_00110D2C wrapper around command table bounds
 *   FUN_00104F88 find/init an MMC backend by numeric ID
 *   FUN_0011C3AC original intrusive backend list lookup
 *
 * Runs only in separate, disposable generic ARM virt processes.
 * Provenance: same SHA-pinned original P3 loader used in V23.
 * No command dispatch, filesystem I/O, controller MMIO, MMC writes,
 * host USB, eMMC access, or actual MPFree kernel boot.
 */
#define MPFREE_V24_WRAPPING 1
#include "vendor_probe_v23.c"

#define V24_CMD_DIRECT ((uintptr_t)0x00110ccdu)
#define V24_CMD_WRAPPER ((uintptr_t)0x00110d2du)
#define V24_MMC_DEVICE ((uintptr_t)0x00104f89u)
#define V24_START_CELL ((uintptr_t)0x00110d3cu)
#define V24_END_CELL ((uintptr_t)0x00110d40u)
#define V24_MULT_CELL ((uintptr_t)0x00110d44u)
#define V24_MMC_BACKEND_A (HEAP+0x1000u)
#define V24_MMC_BACKEND_B (HEAP+0x1800u)
#define V24_COMMAND_STRING (HEAP+0x3000u)

static const char *const v24_commands[]={
    "mmc","mmcinfo","load","ls","ext4load","fatload","ext4ls","fatls",
    "bootz","bootm","source","sysboot","run","env","printenv","setenv",
    "saveenv","help","echo","test","fdt","part","usb","dfu",
    "fastboot","reset","version","crc32","iminfo","imxtract",
    "mpfree_nonexistent_command_xyz","mmc.MPFreeInvalidSuffix"
};
#define V24_NCOMMANDS (sizeof(v24_commands)/sizeof(v24_commands[0]))

static int v24_in_original_image(uint32_t address,size_t n){
    return (uint64_t)address>=BASE &&
           (uint64_t)address+(uint64_t)n <= (uint64_t)BASE+LENGTH;
}
static int v24_original_string(uint32_t where,const char *wanted){
    size_t n=strlen(wanted);
    return v24_in_original_image(where,n+1) &&
           memcmp((void*)(uintptr_t)where,wanted,n+1)==0;
}
static int v24_table(uint32_t *first,uint32_t *last,unsigned *n){
    uint32_t a=get32(V24_START_CELL),b=get32(V24_END_CELL);
    uint32_t magic=get32(V24_MULT_CELL);
    if(a>=b || !v24_in_original_image(a,(size_t)(b-a)) ||
       (b-a)%28u!=0u || (b-a)/28u<5u || (b-a)/28u>512u)
    {
        printf("{\"v24_owner_command_table_error\":\"invalid-or-outside-private-P3\","
               "\"first\":\"0x%08x\",\"last\":\"0x%08x\","
               "\"count_multiplier\":\"0x%08x\"}\n",a,b,magic);
        return 0;
    }
    *first=a;*last=b;*n=(b-a)/28u;
    return 1;
}
/* Independently locate an EXACT name by reading the real owner command
 * structures. This is not a rewrite of the prefix-aware U-Boot lookup:
 * it is an explicit independent cross-check for exact command names. */
static uint32_t v24_exact_descriptor(const char *name,
                                    uint32_t start,unsigned n){
    uint32_t match=0u;
    for(unsigned j=0;j<n;j++){
        uint32_t at=start+j*28u;
        uint32_t text=get32(at);
        if(v24_original_string(text,name)){
            if(match!=0u)return 0u; /* duplicate non-unique table names */
            match=at;
        }
    }
    return match;
}
static int v24_command(const char *test){
    unsigned idx=0;
    if(sscanf(test,"v24_cmd_%u",&idx)!=1 || idx>=V24_NCOMMANDS)
        return result(test,false,"invalid-readonly-command-test");
    uint32_t first=0u,last=0u;
    unsigned count=0u;
    if(!v24_table(&first,&last,&count))
        return result(test,false,"source-command-table-layout-unproven");
    const char *name=v24_commands[idx];
    strcpy((char*)V24_COMMAND_STRING,name);
    uint32_t (*direct)(const char*,uint32_t,unsigned,uint32_t)=
       (uint32_t(*)(const char*,uint32_t,unsigned,uint32_t))V24_CMD_DIRECT;
    uint32_t (*wrapper)(const char*)=(uint32_t(*)(const char*))V24_CMD_WRAPPER;
    v21_phase=31;
    uint32_t found_direct=direct((const char*)V24_COMMAND_STRING,
                                  first,count,0u);
    uint32_t found_wrapper=wrapper((const char*)V24_COMMAND_STRING);
    uint32_t found_exact=v24_exact_descriptor(name,first,count);
    int valid=(found_direct==0u || (
        found_direct>=first && found_direct+28u<=last &&
        ((found_direct-first)%28u)==0u));
    uint32_t handler=valid && found_direct?get32(found_direct+12u):0u;
    int safe_handler=(handler==0u ||
                      v24_in_original_image(handler&~1u,4));
    int wrapper_agrees=(found_direct==found_wrapper);
    int expected_exact=idx<30u;
    int correct=valid && safe_handler && wrapper_agrees &&
                (!expected_exact || found_direct==found_exact) &&
                (idx!=30u || found_direct==0u);
    printf("{\"trace\":\"v24-genuine-native-command-lookup\","
           "\"test\":\"%s\",\"requested\":\"%s\","
           "\"original_registered_table_start\":\"0x%08x\","
           "\"original_registered_table_end\":\"0x%08x\","
           "\"number_registered\":%u,"
           "\"exact_descriptor\":\"0x%08x\","
           "\"owner_direct_lookup\":\"0x%08x\","
           "\"owner_wrapper_lookup\":\"0x%08x\","
           "\"handler_thumb_pointer\":\"0x%08x\","
           "\"handler_was_executed\":false,"
           "\"physical_MMC_access\":false}\n",
           test,name,first,last,count,found_exact,found_direct,
           found_wrapper,handler);
    return result(test,correct,
                  "original-U-Boot-command-descriptor-lookup-never-dispatched");
}
static int v24_setup_mmc_registry(void){
    uint32_t sentinel=get32(V21_SENTINEL_CELL);
    if(!v24_in_original_image(sentinel,4u) ||
       !v21_patch_ram_word(sentinel,(uint32_t)V24_MMC_BACKEND_A))
        return 0;
    memset((void*)V24_MMC_BACKEND_A,0,0x700);
    memset((void*)V24_MMC_BACKEND_B,0,0x700);
    put32(V24_MMC_BACKEND_A,(uint32_t)V24_MMC_BACKEND_B);
    put32(V24_MMC_BACKEND_B,sentinel);
    put32(V24_MMC_BACKEND_A+0xccu,0u);
    put32(V24_MMC_BACKEND_B+0xccu,1u);
    put32(V24_MMC_BACKEND_A+0x14u,0xabad1deau);
    put32(V24_MMC_BACKEND_B+0x14u,0xabad1deau);
    return 1;
}
static int v24_mmc(const char *test){
    unsigned idx=0;
    if(sscanf(test,"v24_mmc_%u",&idx)!=1||idx>=16u)
        return result(test,false,"invalid-MMC-index-experiment");
    unsigned id=idx%4u; /* 0 and 1 present; 2 and 3 absent */
    unsigned force=(idx/4u)%2u;
    unsigned fail=(idx/8u)%2u;
    if(!v24_setup_mmc_registry())
        return result(test,false,"cannot-build-isolated-MMC-registry");
    unsigned init_count=0u,print_count=0u;
    if(!intercept(0x0011daf4u,fail?1u:0u,&init_count) ||
       !intercept(0x0012e96au,0u,&print_count) ||
       !seal())return 4;
    uint32_t (*select)(uint32_t,uint32_t)=
        (uint32_t(*)(uint32_t,uint32_t))V24_MMC_DEVICE;
    v21_phase=32;
    uint32_t owner=select(id,force);
    uint32_t expected=(id==0u)?(uint32_t)V24_MMC_BACKEND_A:
                      (id==1u)?(uint32_t)V24_MMC_BACKEND_B:0u;
    unsigned init_expected=(id<2u)?1u:0u;
    if(fail)expected=0u;
    uint32_t initial_flag=0xabad1deau;
    uint32_t observed_flag=(id<2u)?
       get32((id==0?V24_MMC_BACKEND_A:V24_MMC_BACKEND_B)+0x14u):0u;
    int valid=owner==expected &&
              calls(init_count)==init_expected &&
              observed_flag==(force?0u:initial_flag);
    printf("{\"trace\":\"v24-genuine-MMC-index-to-registered-backend\","
           "\"test\":\"%s\",\"requested_Uboot_device_id\":%u,"
           "\"synthetic_registry_has_id\":%s,"
           "\"forced_reinit\":%u,\"mock_init_failure\":%u,"
           "\"backend_return\":\"0x%08x\","
           "\"expected_backend\":\"0x%08x\","
           "\"real_MMC_lookup_and_wrapper_executed\":true,"
           "\"device_initialization_STUB_calls\":%u,"
           "\"physical_SD_or_eMMC_selected\":false}\n",
           test,id,id<2u?"true":"false",force,fail,
           owner,expected,calls(init_count));
    return result(test,valid,
                  "real-owner-MMC-ID-selector-with-synthetic-list-and-no-media");
}
int main(int argc,char **argv){
    if(argc!=3){
        fputs("usage: vendor_probe_v24 CASE private-SHA-pinned-P3.raw\n",stderr);
        return 2;
    }
    if(strncmp(argv[1],"v24_",4u)!=0)
        return mpfree_v23_legacy_main(argc,argv);
    alarm(15);
    if(!v21_hooks(argv[1])||!prepare(argv[2]))return 3;
    if(get32(0x0012284cu)!=FLAG || get32(0x00122928u)!=FLAG)
        return result(argv[1],false,"original-owner-permission-global-mismatch");
    if(!strncmp(argv[1],"v24_cmd_",8))
        return v24_command(argv[1]);
    if(!strncmp(argv[1],"v24_mmc_",8))
        return v24_mmc(argv[1]);
    return result(argv[1],false,"unrecognized-V24-experiment");
}
