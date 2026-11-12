/* MPFree V15: execute ONLY the hash-verified 22-byte vendor Fastboot
 * permission function in a disposable ARM QEMU guest, with mock RAM.
 *
 * P3 code page is extracted from a SHA-verified owner Ghidra image, and
 * staged on a PRIVATE disposable read-only QEMU ext4 fixture. No full vendor
 * bootloader, USB stack, updater or peripheral is executed.
 *
 * The response function at 0x00122734 is REPLACED IN GUEST MEMORY by BX LR.
 * This makes the denied branch non-I/O and avoids touching vendor USB code.
 * Only guest virtual memory 0x00122000 and 0x0014d000 is mapped.
 */
#define _GNU_SOURCE
#include <sys/mman.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <signal.h>
#include <errno.h>
#ifndef MAP_FIXED_NOREPLACE
#define MAP_FIXED_NOREPLACE 0x100000
#endif
#define PAGE_ADDR ((uintptr_t)0x00122000u)
#define GLOBAL_PAGE ((uintptr_t)0x0014d000u)
#define FLAG_ADDR ((uintptr_t)0x0014d690u)
#define RESPONSE_ADDR ((uintptr_t)0x00122734u)
#define ENTRY_THUMB ((uintptr_t)0x00122835u)
#define POINTER_LITERAL ((uintptr_t)0x0012284cu)
static unsigned checks,failed;
static void test(const char *id,int ok){
    printf("{\"test\":\"%s\",\"pass\":%s,\"kind\":\"actual_vendor_function_slice\"}\n",
           id,ok?"true":"false");
    fflush(stdout);
    ++checks; if(!ok)++failed;
}
static int load_page(const char *file,unsigned char out[4096]){
    FILE *f=fopen(file,"rb");
    if(!f)return 0;
    size_t n=fread(out,1,4096,f);
    int extra=fgetc(f);
    fclose(f);
    return n==4096&&extra==EOF;
}
static void *map_exact(uintptr_t addr,int protection){
    void *p=mmap((void*)addr,4096,protection,
                 MAP_PRIVATE|MAP_ANONYMOUS|MAP_FIXED_NOREPLACE,-1,0);
    if(p==MAP_FAILED || (uintptr_t)p!=addr){
        if(p!=MAP_FAILED)munmap(p,4096);
        return NULL;
    }
    return p;
}
int main(int argc,char **argv){
    if(argc!=2){fputs("usage: v15_permission_lab <private_owner_4k_code_page>\n",stderr);return 2;}
    alarm(7);
    unsigned char source[4096];
    if(!load_page(argv[1],source)){fputs("Missing 4096-byte verified function page\n",stderr);return 2;}
    void *code=map_exact(PAGE_ADDR,PROT_READ|PROT_WRITE);
    void *state=map_exact(GLOBAL_PAGE,PROT_READ|PROT_WRITE);
    if(!code||!state){fprintf(stderr,"VM address unavailable: errno=%d\n",errno);return 3;}
    memcpy(code,source,4096);
    uint32_t global_ptr=0;
    memcpy(&global_ptr,(char*)code+(POINTER_LITERAL-PAGE_ADDR),sizeof(global_ptr));
    test("genuine_literal_points_to_mockable_authorization_global",global_ptr==FLAG_ADDR);
    if(global_ptr!=FLAG_ADDR){munmap(code,4096);munmap(state,4096);return 4;}
    /* Neutralize only the denied-path USB response function.
       Thumb BX LR = 0x4770 in little-endian encoding. */
    unsigned char *response=(unsigned char*)code+(RESPONSE_ADDR-PAGE_ADDR);
    response[0]=0x70;response[1]=0x47;
    __builtin___clear_cache((char*)code,(char*)code+4096);
    if(mprotect(code,4096,PROT_READ|PROT_EXEC)!=0){
        perror("mprotect");return 5;
    }
    uint32_t (*permission)(void)=(uint32_t(*)(void))ENTRY_THUMB;
    volatile uint32_t *flag=(volatile uint32_t*)FLAG_ADDR;
    *flag=1u;
    uint32_t allowed=permission();
    test("actual_arm_function_authorized_value_one_returns_true",allowed==1u);
    test("authorized_function_does_not_mutate_flag",*flag==1u);
    *flag=0u;
    uint32_t denied=permission();
    test("actual_arm_function_denied_value_zero_returns_false",denied==0u);
    test("denied_function_does_not_mutate_flag",*flag==0u);
    *flag=2u;
    uint32_t nonzero=permission();
    test("actual_arm_function_nonzero_returns_true",nonzero==1u);
    test("nonzero_function_does_not_mutate_flag",*flag==2u);
    printf("{\"summary\":\"real_v15_function_slice_in_mocked_arm_guest\","
           "\"cases\":%u,\"failed\":%u,\"physical_device_access\":false,"
           "\"other_vendor_functions_executed\":false}\n",checks,failed);
    puts(failed?"MPFREE_V15_VENDOR_FUNCTION_FAIL":"MPFREE_V15_VENDOR_FUNCTION_PASS");
    fflush(stdout);
    munmap(code,4096);munmap(state,4096);
    return failed?1:0;
}
