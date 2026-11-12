/* MPFree V20: execute actual U-Boot storage-backend and command parser
 * inside isolated generic ARM virt with guest-only synthetic state.
 *
 * The real backend callback is REPLACED in private guest RAM, never executed.
 * The actual code under test is FUN_0010b74c, FUN_0011c3ec (backend offset),
 * mpf_run_command_candidate, and FUN_001095f8. Storage operations and
 * command dispatch are stubbed. No physical USB/MMC/eMMC or kernel boot.
 */
#define MPFREE_V18_WRAPPING 1
#include "vendor_probe_v17.c"
#define V20_BACKEND ((uintptr_t)0x0010b74du)
#define V20_CMD ((uintptr_t)0x00110485u)
#define V20_FAKE_BACKEND (HEAP+0x5000u)
#define V20_CMD_TEXT (HEAP+0x5800u)
static int v20_return_pointer(unsigned slot, uintptr_t ptr){
    if(slot>=slots)return 0;
    uintptr_t where=TRAMP+64u*(uintptr_t)slot;
    write16(where+8,0x4803u); /* LDR r0,[pc,#12] */
    write32(where+24,(uint32_t)ptr);
    return 1;
}
static int v20_storage(const char *name){
    unsigned variant=0;
    if(sscanf(name,"v20_storage_%u",&variant)!=1||variant>5)
        return result(name,false,"invalid-storage-branch");
    unsigned lookup=0,initialize=0,read_image=0,geometry=0;
    unsigned final_callback=0,align=0,done=0,log=0,baseline=0;
    if(!intercept(0x0010b732u,0,&baseline) ||
       !intercept(0x0011c3acu,0,&lookup) ||
       !intercept(0x0011daf4u,variant==1?1u:0u,&initialize) ||
       !intercept(0x0010cdf4u,variant==2?1u:0u,&read_image) ||
       !intercept(0x0010b728u,variant==3?1u:0u,&geometry) ||
       !intercept(0x0012ee08u,16u,&align) ||
       !intercept(0x0010d14cu,0,&done) ||
       !intercept(0x0012e96au,0,&log) ||
       !intercept(0x0012ccd0u,(variant==5)?0u:16u,&final_callback))
        return 4;
    memset((void*)V20_FAKE_BACKEND,0,1024);
    write32(V20_FAKE_BACKEND+0x6cu,0x200u);
    /* Real FUN_0011c3ec(b) returns b+0xc8, and actual FUN_0010b74c
       loads the indirect method from [b+0xc8+0x68] = [b+0x130].
       This is the *specific* storage-dispatch offset to test. */
    write32(V20_FAKE_BACKEND+0x130u,
            (uint32_t)(TRAMP+64u*(uintptr_t)final_callback+1u));
    if(!v20_return_pointer(lookup,variant==0?0u:V20_FAKE_BACKEND))
        return 4;
    if(!seal())return 4;
    int (*vendor_backend)(void)=(int(*)(void))V20_BACKEND;
    int code=vendor_backend();
    unsigned expect_callback=(variant==4||variant==5)?1u:0u;
    unsigned expected_code=(variant==4)?0u:1u;
    bool ok=calls(final_callback)==expect_callback && code==(int)expected_code;
    ok=ok&&calls(lookup)==1&&calls(initialize)==(unsigned)(variant!=0);
    ok=ok&&calls(read_image)==(unsigned)(variant>1);
    ok=ok&&calls(geometry)==(unsigned)(variant>2);
    ok=ok&&calls(done)==(unsigned)(variant>=4);
    printf("{\"trace\":\"v20-owner-storage-backend\",\"case\":\"%s\","
           "\"backend_lookup\":%u,\"initialize\":%u,\"read\":%u,"
           "\"geometry\":%u,\"dispatch_callback\":%u,"
           "\"expected_callback\":%u,\"image_status_calls\":%u,"
           "\"vendor_return\":%d,\"expected_return\":%u,"
           "\"object_offset\":\"0x130\"}\n",
           name,calls(lookup),calls(initialize),calls(read_image),
           calls(geometry),calls(final_callback),expect_callback,
           calls(done),code,expected_code);
    return result(name,ok,"genuine-indirect-storage-callback-isolated-in-guest");
}
static int v20_command(const char *name){
    unsigned variant=0,mode=0;
    if(sscanf(name,"v20_command_%u_%u",&variant,&mode)!=2 ||
       variant>7 || mode>1)
        return result(name,false,"invalid-command-branch");
    const char *candidates[]={
       NULL,"","echo mpfree","echo mpfree\n","mmc dev 1",
       "ext4load mmc 1:1 0x02000000 /boot/zImage",
       "bootz 0x02000000 - 0x01f00000","saveenv"
    };
    const char *cmd=candidates[variant];
    uintptr_t copied=(HEAP+0x5900u);
    uintptr_t output=(HEAP+0x7000u);
    if(cmd){
        size_t size=strlen(cmd);
        if(size>200)return result(name,false,"unexpected-case-size");
        memcpy((void*)copied,cmd,size+1);
    }
    memset((void*)output,0,2048);
    unsigned command_dispatch=0,allocate=0,release=0,storage=0;
    if(!intercept(0x00109b68u,0,&command_dispatch) ||
       !intercept(0x00108de0u,0,&allocate) ||
       !intercept(0x0010d588u,0,&release) ||
       !intercept(0x0010b74cu,0,&storage))
        return 4;
    if(!v20_return_pointer(allocate,output) || !seal())return 4;
    ((void(*)(const char*,uint32_t))V20_CMD)(
        cmd?(const char*)copied:NULL,mode?4u:0u);
    unsigned expected=variant>=2?1u:0u;
    bool ok=calls(command_dispatch)==expected&&calls(storage)==0;
    /* A parsed but non-executed saveenv word is NOT evidence that
       physical U-Boot command execution would avoid persistence. */
    printf("{\"trace\":\"v20-command-parser\",\"case\":\"%s\","
           "\"dispatch_calls\":%u,\"expected_dispatch_calls\":%u,"
           "\"guest_allocator\":%u,\"guest_free\":%u,"
           "\"persistent_backend_callbacks\":%u,\"dispatch_was_stUBBED\":true}\n",
           name,calls(command_dispatch),expected,calls(allocate),
           calls(release),calls(storage));
    return result(name,ok,"genuine-U-Boot-parser-reached-synthetic-dispatch-only");
}
int main(int argc,char **argv){
    if(argc!=3){
        fputs("usage: vendor_probe_v20 CASE private-SHA-pinned-P3\n",stderr);
        return 2;
    }
    if(strncmp(argv[1],"v20_",4))return mpfree_v17_legacy_main(argc,argv);
    alarm(14);
    if(!prepare(argv[2]))return 3;
    if(get32(0x0012284cu)!=FLAG||get32(0x00122928u)!=FLAG)
        return result(argv[1],false,"owner-authentication-pointer-mismatch");
    if(!strncmp(argv[1],"v20_storage_",12))return v20_storage(argv[1]);
    if(!strncmp(argv[1],"v20_command_",12))return v20_command(argv[1]);
    return result(argv[1],false,"unrecognized-V20-case");
}
