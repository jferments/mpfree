/* MPFree V23: actual U-Boot MMC command-table indirection and controller
 * configuration routines in ARM virt, using synthetic guest RAM only.
 *
 * Owner P3 SHA pinned by the parent VM runner. Real static table:
 * [0x0012f8f4] -> Thumb 0x0011c4e9 (hardware request)
 * [0x0012f8f8] -> Thumb 0x0011c913 (controller configuration)
 * FUN_0011ca78 dispatches via:
 *     b = *(backend+8); table = *(b+4); callback = *(table+0);
 *
 * There is NO host USB/eMMC, real SD/MMC register, DMA, boot, saveenv,
 * or physical MPC access. The hardware command is intercepted BEFORE
 * entering its real MMIO body. We run its genuine dispatch and separately
 * run its genuine controller-configuration entry against FAKE registers.
 */
#define MPFREE_V23_WRAPPING 1
#include "vendor_probe_v22.c"

#define V23_DISPATCH ((uintptr_t)0x0011ca79u)
#define V23_CFG ((uintptr_t)0x0011c913u)
#define V23_DRIVER_TABLE ((uintptr_t)0x0012f8f4u)
#define V23_OWNER_COMMAND ((uintptr_t)0x0011c4e9u)
#define V23_OWNER_CONFIG ((uintptr_t)0x0011c913u)
#define V23_BACKEND (HEAP+0x1000u)
#define V23_DEVICE (HEAP+0x2000u)
#define V23_TABLE (HEAP+0x3000u)
#define V23_CMD (HEAP+0x4000u)
#define V23_PAYLOAD (HEAP+0x4800u)
#define V23_MMIO (HEAP+0x5000u)
#define V23_CONFIG (HEAP+0x6000u)

static struct {
    uint32_t called,backend,command,data,returned;
} v23_observed;

__attribute__((noinline))
static uint32_t v23_nonwriting_command(uint32_t backend,
    uint32_t command,uint32_t data){
    v23_observed.called++;
    v23_observed.backend=backend;
    v23_observed.command=command;
    v23_observed.data=data;
    return v23_observed.returned;
}
static int v23_check_table(void){
    uint32_t first=get32(V23_DRIVER_TABLE);
    uint32_t second=get32(V23_DRIVER_TABLE+4u);
    printf("{\"v23_owner_table\":{\"address\":\"0x0012f8f4\","
           "\"first\":\"0x%08x\",\"second\":\"0x%08x\","
           "\"raw_image_pinned\":true}}\n",first,second);
    return first==V23_OWNER_COMMAND && second==V23_OWNER_CONFIG;
}
static int v23_dispatch(const char *name){
    unsigned index=0;
    if(sscanf(name,"v23_dispatch_%u",&index)!=1||index>=48u)
        return result(name,false,"invalid-driver-dispatch-case");
    memset((void*)V23_BACKEND,0,0x500);
    memset((void*)V23_DEVICE,0,0x500);
    memset((void*)V23_CMD,0,0x200);
    memset((void*)V23_PAYLOAD,0x5a,0x200);
    memset(&v23_observed,0,sizeof(v23_observed));
    put32(V23_BACKEND+8u,(uint32_t)V23_DEVICE);
    uint32_t variant=index%16u;
    uint32_t code=variant==0u?0x18u:variant==1u?0x19u:
                  variant==2u?0x0cu:variant==3u?0x26u:
                  variant==4u?0x15u:variant==5u?0x0du:
                  variant==6u?0x10u:variant==7u?0x01u:
                  variant==8u?0x20u:variant==9u?0x23u:
                  variant==10u?0x24u:variant==11u?0x00u:
                  variant==12u?0xffu:variant==13u?0x100u:
                  variant==14u?0x200u:0x31u;
    put32(V23_CMD,code);
    uint32_t ret= index<16u? (variant%3u==0u?0u:
                              variant%3u==1u?1u:0xffffffedu)
                          : index<32u?0u:0xffffffedu;
    uint32_t (*dispatch)(uint32_t,uint32_t,uint32_t)=
        (uint32_t(*)(uint32_t,uint32_t,uint32_t))V23_DISPATCH;
    if(index<16u){
        /* Use the REAL owner device-driver table from firmware, then
         * replace ONLY its hardware-command destination inside guest RAM. */
        put32(V23_DEVICE+4u,(uint32_t)V23_DRIVER_TABLE);
        unsigned trap=0;
        if(!intercept(0x0011c4e8u,ret,&trap)||!seal())
            return result(name,false,"cannot-intercept-owner-hardware-callback");
        v21_phase=21;
        uint32_t observed=dispatch((uint32_t)V23_BACKEND,(uint32_t)V23_CMD,
                                   (uint32_t)V23_PAYLOAD);
        printf("{\"trace\":\"v23-real-owner-driver-table\","
               "\"test\":\"%s\",\"owner_table\":\"0x0012f8f4\","
               "\"command_word\":%u,\"hardware_callback_STUB_calls\":%u,"
               "\"actual_dispatch_return\":%u,\"expected\":%u,"
               "\"hardware_MMIO_executed\":false}\n",
               name,code,calls(trap),observed,ret);
        return result(name,calls(trap)==1u && observed==ret,
                      "real-U-Boot-driver-table-to-intercepted-controller-command");
    }
    /* Same genuine FUN_0011ca78 dispatch, but a freshly built synthetic
     * table to verify argument forwarding and callback ABI. */
    put32(V23_DEVICE+4u,(uint32_t)V23_TABLE);
    put32(V23_TABLE,(uint32_t)(uintptr_t)&v23_nonwriting_command);
    v23_observed.returned=ret;
    v21_phase=22;
    uint32_t got=dispatch((uint32_t)V23_BACKEND,(uint32_t)V23_CMD,
                           (uint32_t)V23_PAYLOAD);
    bool ok=v23_observed.called==1u && got==ret &&
            v23_observed.command==(uint32_t)V23_CMD &&
            v23_observed.data==(uint32_t)V23_PAYLOAD;
    printf("{\"trace\":\"v23-driver-dispatch-argument-ABI\","
           "\"test\":\"%s\",\"command_word\":%u,"
           "\"callback_arg0\":\"0x%08x\","
           "\"callback_arg1\":\"0x%08x\","
           "\"callback_arg2\":\"0x%08x\","
           "\"callback_calls\":%u,\"return\":%u,\"expected\":%u,"
           "\"arg0_equals_backend\":%s}\n",
           name,code,v23_observed.backend,v23_observed.command,
           v23_observed.data,v23_observed.called,got,ret,
           v23_observed.backend==(uint32_t)V23_BACKEND?"true":"false");
    return result(name,ok,
                  "actual-owner-tail-dispatch-to-fake-driver-with-verified-r1-r2");
}
static int v23_config(const char *name){
    unsigned variant=0;
    if(sscanf(name,"v23_config_%u",&variant)!=1||variant>=12u)
        return result(name,false,"invalid-controller-configuration-case");
    memset((void*)V23_CONFIG,0,0x500);
    memset((void*)V23_DEVICE,0,0x500);
    memset((void*)V23_MMIO,0,0x500);
    put32(V23_CONFIG+0x10u,(uint32_t)V23_DEVICE);
    put32(V23_DEVICE+4u,(uint32_t)V23_MMIO);
    put32(V23_CONFIG+0x20u,0u);
    put32(V23_DEVICE+0x14u,0u); /* skip unmocked reset path */
    put32(V23_DEVICE+0x38u,0u); /* no driver callback */
    uint32_t selected=variant%3u==0u?4u:variant%3u==1u?8u:0u;
    uint32_t bitorclear=variant>=6u;
    put32(V23_CONFIG+0x1cu,selected);
    put32(V23_CONFIG+0x140u,bitorclear);
    uint32_t old=variant%2u?0x10000u:0u;
    put32(V23_MMIO+0x74u,old);
    uint32_t canary=0xa5a55a5au;
    put32(V23_MMIO+0x2cu,canary);
    v21_phase=23;
    void (*configure)(uint32_t)=(void(*)(uint32_t))V23_CFG;
    configure((uint32_t)V23_CONFIG);
    uint32_t expected_mode=selected==4u?1u:selected==8u?0x10000u:0u;
    uint32_t observed_mode=get32(V23_MMIO+0x18u);
    uint32_t observed_reg=get32(V23_MMIO+0x74u);
    uint32_t expected_reg=(old&~0x10000u)|(bitorclear?0x10000u:0u);
    bool ok=observed_mode==expected_mode &&
            observed_reg==expected_reg &&
            get32(V23_MMIO+0x2cu)==canary;
    printf("{\"trace\":\"v23-original-controller-config\","
           "\"test\":\"%s\",\"mode_input\":%u,\"register18\":%u,"
           "\"expected_register18\":%u,"
           "\"register74\":\"0x%08x\",\"expected_register74\":\"0x%08x\","
           "\"command_register_unchanged\":%s,"
           "\"synthetic_guest_registers\":true}\n",
           name,selected,observed_mode,expected_mode,observed_reg,expected_reg,
           get32(V23_MMIO+0x2cu)==canary?"true":"false");
    return result(name,ok,
                  "real-controller-configuration-routine-wrote-only-fake-registers");
}
int main(int argc,char **argv){
    if(argc!=3){
        fputs("usage: vendor_probe_v23 CASE owner-P3-pinned-private.raw\n",stderr);
        return 2;
    }
    if(strncmp(argv[1],"v23_",4)!=0)
        return mpfree_v22_legacy_main(argc,argv);
    alarm(15);
    if(!v21_hooks(argv[1])||!prepare(argv[2]))return 3;
    if(get32(0x0012284cu)!=FLAG || get32(0x00122928u)!=FLAG)
        return result(argv[1],false,"original-authorization-global-invalid");
    if(!v23_check_table())
        return result(argv[1],false,"original-driver-table-pointers-mismatch");
    if(!strncmp(argv[1],"v23_dispatch_",13))
        return v23_dispatch(argv[1]);
    if(!strncmp(argv[1],"v23_config_",11))
        return v23_config(argv[1]);
    return result(argv[1],false,"unknown-V23-case");
}
