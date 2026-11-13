/* MPFree V22 — execute ORIGINAL ARM division + vendor storage callbacks.
 *
 * Two independent original U-Boot execution paths:
 *  1. FUN_0010b74c -> UNMODIFIED ARM-state unsigned division 0x0012ee08
 *     -> the true indirect interface pointer at b+0x130 -> instrumented,
 *     non-writing guest C callback.
 *  2. REGISTERED vendor write implementation 0x0011de50, whose actual
 *     pointer is present in owner P3 literal cell 0x0011d95c. This calls
 *     the true owner backend registry and real chunking/command-selection
 *     code while ONLY low-level data transport, readiness and wait/status
 *     callbacks are mocked in guest RAM.
 *
 * Device-independent transfer metadata is real owner code; all hardware
 * contact (MMC, USB, RockUSB, DMA, eMMC) and persistent writes excluded.
 * The image is SHA-pinned and lives in a disposable ARM-virt guest.
 */
#define MPFREE_V22_WRAPPING 1
#include "vendor_probe_v21.c"

#define V22_DIVISION ((uintptr_t)0x0012ee08u) /* ARM mode: low bit = 0 */
#define V22_OWNER_WRAPPER ((uintptr_t)0x0010b74du) /* Thumb */
#define V22_OWNER_WRITE ((uintptr_t)0x0011de51u) /* Thumb */
#define V22_WRITE_POINTER_LITERAL ((uintptr_t)0x0011d95cu)
#define V22_ERASE_POINTER_LITERAL ((uintptr_t)0x0011d960u)
#define V22_INPUT_BACKEND (HEAP+0x4400u)
#define V22_WRITE_BACKEND (HEAP+0x3000u)
#define V22_INPUT_DEVICE (HEAP+0x5400u)
#define V22_GUEST_DATA (HEAP+0x6000u)

static struct {
    unsigned calls;
    uint32_t interface_addr,start,size,data_addr,returned;
    uint32_t return_value;
} v22_cb;

/* C callback, not a guessed Thumb/ARM binary trampoline. The original
 * owner U-Boot calls this through the genuine b+0x130 interface field.
 * Its ABI is compiled for ARM by arm-linux-gnueabihf-gcc. */
__attribute__((noinline))
static uint32_t v22_synthetic_media_write(uint32_t interface_addr,
    uint32_t sector_start,uint32_t sector_count,uint32_t source_addr){
    v22_cb.calls++;
    v22_cb.interface_addr=interface_addr;
    v22_cb.start=sector_start;
    v22_cb.size=sector_count;
    v22_cb.data_addr=source_addr;
    v22_cb.returned=v22_cb.return_value;
    return v22_cb.return_value;
}
static uint32_t v22_udiv(uint32_t dividend,uint32_t divisor){
    uint32_t (*f)(uint32_t,uint32_t)=(uint32_t (*)(uint32_t,uint32_t))V22_DIVISION;
    return f(dividend,divisor);
}
static int v22_division(const char *name){
    unsigned variant=0;
    static const struct{uint32_t a,b;} cases[]={
      {0u,1u},{1u,1u},{8192u,512u},{0x18000u,512u},
      {0x18000u,1024u},{8193u,512u},{0xffffffffu,512u},
      {0x80000000u,3u},{0xfffffffeu,255u},{65535u,65536u},
      {0x70000000u,4096u},{32768u,7u}
    };
    if(sscanf(name,"v22_div_%u",&variant)!=1 || variant>=12)
        return result(name,false,"bad-division-case");
    uint32_t a=cases[variant].a,b=cases[variant].b;
    v21_phase=11;
    uint32_t actual=v22_udiv(a,b);
    uint32_t wanted=a/b;
    printf("{\"trace\":\"v22-original-arm-division\",\"test\":\"%s\","
           "\"function\":\"0x0012ee08\",\"ARM_state\":true,"
           "\"numerator\":%u,\"denominator\":%u,\"result\":%u,\"expected\":%u}\n",
           name,a,b,actual,wanted);
    return result(name,actual==wanted,
                  "original-unaltered-ARM-division-not-a-Thumb-stub");
}
static int v22_wrapper(const char *name){
    unsigned variant=0;
    if(sscanf(name,"v22_wrapper_%u",&variant)!=1 || variant>7)
        return result(name,false,"bad-wrapper-case");
    /* Four alignment-dependent successes; two failure counts and
     * two same-interface controls. These are not duplicated V21 cases:
     * V21 never reached this callback due to the ARM/Thumb patch bug. */
    uint32_t blocks=256u << (variant%4u);
    uint32_t starting=(8192u+blocks-1u)/blocks;
    uint32_t length=(0x18000u+blocks-1u)/blocks;
    unsigned reset=0,lookup=0,ready=0,buffer=0,log=0,completion=0;
    if(!intercept(0x0010b732u,0,&reset) ||
       !intercept(0x0011c3acu,0,&lookup) ||
       !intercept(0x0011daf4u,0,&ready) ||
       !intercept(0x0010cdf4u,0,&buffer) ||
       !intercept(0x0012e96au,0,&log) ||
       !intercept(0x0010d14cu,0,&completion))
        return 4;
    memset((void*)V22_INPUT_BACKEND,0,0x600);
    put32(V22_INPUT_BACKEND+0x6cu,blocks);
    put32(V22_INPUT_BACKEND+0xccu,11u);
    put32(V22_INPUT_BACKEND+0x130u,
          (uint32_t)(uintptr_t)&v22_synthetic_media_write);
    if(!v20_return_pointer(lookup,V22_INPUT_BACKEND) || !seal())
        return 4;
    memset(&v22_cb,0,sizeof(v22_cb));
    v22_cb.return_value=(variant==4)?0u:
                        (variant==5)?starting+1u:starting;
    if(variant==6)v22_cb.return_value=starting;
    if(variant==7)v22_cb.return_value=starting;
    /* No intercept for the genuine ARM-mode division routine; a patched
       Thumb stub at its EVEN 0x0012ee08 entry caused V20/V21 SIGILL. */
    v21_phase=12;
    int(*owner)(void)=(int(*)(void))V22_OWNER_WRAPPER;
    int rc=owner();
    uint32_t expected_rc=(v22_cb.return_value==starting)?0u:1u;
    bool ok=(v22_cb.calls==1 &&
             v22_cb.interface_addr==V22_INPUT_BACKEND+0xc8u &&
             v22_cb.start==length && v22_cb.size==starting &&
             v22_cb.data_addr!=0u &&
             calls(lookup)==1 && calls(ready)==1 && calls(buffer)==1 &&
             rc==(int)expected_rc);
    printf("{\"trace\":\"v22-genuine-storage-wrapper\",\"test\":\"%s\","
           "\"real_arm_division\":true,\"sector_bytes\":%u,"
           "\"expected_start_sector\":%u,\"observed_start_sector\":%u,"
           "\"expected_block_count\":%u,\"observed_block_count\":%u,"
           "\"callback_calls\":%u,\"callback_return\":%u,"
           "\"expected_return\":%u,\"vendor_return\":%d,"
           "\"destination_is_synthetic_guest_RAM\":true}\n",
           name,blocks,length,v22_cb.start,starting,v22_cb.size,
           v22_cb.calls,v22_cb.return_value,expected_rc,rc);
    return result(name,ok,
                  "original-owner-wrapper-and-arm-divide-called-nonwriting-C-callback");
}
static int v22_install_registry(uint32_t block_id,int missing){
    uintptr_t sentinel=get32(V21_SENTINEL_CELL);
    if(sentinel<BASE || sentinel>=BASE+IMAGE_MAP)return 0;
    memset((void*)V22_WRITE_BACKEND,0,0x600);
    memset((void*)V22_INPUT_DEVICE,0,0x100);
    put32(V22_WRITE_BACKEND,(uint32_t)sentinel);
    put32(V22_WRITE_BACKEND+0xccu,11u);
    put32(V22_WRITE_BACKEND+0x08u,(uint32_t)V22_INPUT_DEVICE);
    put32(V22_INPUT_DEVICE+0x18u,block_id);
    put32(V22_WRITE_BACKEND+0x6cu,512u);
    put32(V22_WRITE_BACKEND+0xd8u,64u); /* interface+0x10 capacity */
    put32(V22_WRITE_BACKEND+0x18u,1u); /* sector-addressed media */
    return v21_patch_ram_word(sentinel,missing?(uint32_t)sentinel:
                                    (uint32_t)V22_WRITE_BACKEND);
}
static int v22_registered_write(const char *name){
    unsigned variant=0;
    if(sscanf(name,"v22_write_%u",&variant)!=1 || variant>=9)
        return result(name,false,"bad-real-write-case");
    uint32_t thumb_fn=get32(V22_WRITE_POINTER_LITERAL);
    uint32_t erase_fn=get32(V22_ERASE_POINTER_LITERAL);
    if(thumb_fn!=V22_OWNER_WRITE)
        return result(name,false,"owner-literal-does-not-point-to-concrete-write");
    if((erase_fn&~1u)!=0x0011dd4cu)
        return result(name,false,"owner-literal-does-not-point-to-erase");
    uint32_t start=0,count=1,max_chunk=2u;
    if(variant==1){count=5;max_chunk=2u;}
    if(variant==2){start=63;count=2;}
    if(variant==3){count=1;} /* backend absent */
    if(variant==6){count=0;}
    if(variant==7){count=3;max_chunk=1u;}
    if(variant==8){start=64;count=2;}
    if(!v22_install_registry(max_chunk,variant==3))
        return result(name,false,"synthetic-owner-list-install-failed");
    memset((void*)V22_GUEST_DATA,0xa5,4096);
    unsigned ready=0,capacity=0,transport=0,wait=0,log=0;
    if(!intercept(0x00116198u,0,&ready) ||
       !intercept(0x0011cc4cu,variant==4?1u:0u,&capacity) ||
       !intercept(0x0011ca78u,variant==5?1u:0u,&transport) ||
       !intercept(0x0011cb24u,0,&wait) ||
       !intercept(0x0012e96au,0,&log) || !seal())
        return 4;
    /* Pass the real registered interface pointer, not the parent node.
       This invokes genuine U-Boot write chunking/limit/command logic,
       but the lower-level command transport is a zero-I/O stub. */
    v21_phase=13;
    uint32_t(*write_fn)(uint32_t,uint32_t,uint32_t,uint32_t)=
         (uint32_t(*)(uint32_t,uint32_t,uint32_t))V22_OWNER_WRITE;
    uint32_t observed=write_fn((uint32_t)(V22_WRITE_BACKEND+0xc8u),
                               start,count,(uint32_t)V22_GUEST_DATA);
    uint32_t expected=(variant==2||variant==3||variant==4||
                       variant==5||variant==8)?0u:count;
    bool ok=(observed==expected);
    if(variant==0)ok=ok&&calls(transport)==1&&calls(wait)==1;
    if(variant==1)ok=ok&&calls(transport)==5&&calls(wait)==3;
    if(variant==6)ok=ok&&calls(transport)==0;
    if(variant==7)ok=ok&&calls(transport)==3&&calls(wait)==3;
    if(variant==3)ok=ok&&calls(transport)==0;
    if(variant==2||variant==8)ok=ok&&calls(transport)==0;
    printf("{\"trace\":\"v22-registered-actual-owner-block-write\","
           "\"test\":\"%s\",\"literal\":\"0x%08x\","
           "\"erase_literal\":\"0x%08x\",\"sector_start\":%u,"
           "\"sectors_requested\":%u,\"max_chunk\":%u,"
           "\"real_owner_blocks_reported\":%u,\"expected_blocks\":%u,"
           "\"low_level_transport_STUB_calls\":%u,\"wait_STUB_calls\":%u,"
           "\"capacity_STUB_calls\":%u,\"ready_STUB_calls\":%u,"
           "\"real_media_write_performed\":false}\n",
           name,thumb_fn,erase_fn,start,count,max_chunk,observed,expected,
           calls(transport),calls(wait),calls(capacity),calls(ready));
    return result(name,ok,
        "actual-U-Boot-registered-write-handler-with-low-level-transport-stubbed");
}
int main(int argc,char **argv){
    if(argc!=3){
        fputs("usage: vendor_probe_v22 CASE SHA-verified-owner-P3.raw\n",stderr);
        return 2;
    }
    if(strncmp(argv[1],"v22_",4))return mpfree_v21_legacy_main(argc,argv);
    alarm(14);
    if(!v21_hooks(argv[1]) || !prepare(argv[2]))return 3;
    if(get32(0x0012284cu)!=FLAG || get32(0x00122928u)!=FLAG)
        return result(argv[1],false,"mismatched-owner-authorization-global");
    if(!strncmp(argv[1],"v22_div_",8))return v22_division(argv[1]);
    if(!strncmp(argv[1],"v22_wrapper_",12))return v22_wrapper(argv[1]);
    if(!strncmp(argv[1],"v22_write_",10))return v22_registered_write(argv[1]);
    return result(argv[1],false,"unknown-v22-case");
}
