/* MPFree V16: offline owner-ROM function experiments inside isolated ARM virt.
 * Executes bounded verified U-Boot functions with synthetic RAM and harmless
 * USB-output stubs. NEVER accesses USB, eMMC, updater, or physical MPC.
 * Raw input is a PRIVATE, SHA-pinned export; do not publish it.
 */
#define _GNU_SOURCE
#include <stdint.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
#include <signal.h>
#include <errno.h>
#ifndef MAP_FIXED_NOREPLACE
#define MAP_FIXED_NOREPLACE 0x100000
#endif
#define BASE ((uintptr_t)0x00100000u)
#define LENGTH 363649u
#define IMAGE_MAP 0x59000u
#define HEAP ((uintptr_t)0x00600000u)
#define HEAP_MAP 0x10000u
#define FLAG ((uintptr_t)0x0014d690u)
#define RESPONSE ((uintptr_t)0x00122734u)
#define COMPARE ((uintptr_t)0x0012dc79u)
#define PERMISSION ((uintptr_t)0x00122835u)
#define OEM ((uintptr_t)0x00122855u)
#define CLASSIFY ((uintptr_t)0x0010e391u)
#define FDT_CHECK ((uintptr_t)0x00127a1du)
#define BOOT ((uintptr_t)0x00122965u)
#define USB_EXIT ((uintptr_t)0x00121dd1u)
#define RECEIVE ((uintptr_t)0x00122a11u)
#define USB_DATA ((uintptr_t)0x00800000u)
static void* map_at(uintptr_t addr, size_t length) {
    void *p=mmap((void*)addr,length,PROT_READ|PROT_WRITE,
        MAP_PRIVATE|MAP_ANONYMOUS|MAP_FIXED_NOREPLACE,-1,0);
    if(p==MAP_FAILED || (uintptr_t)p!=addr) {
        fprintf(stderr,"map at %08lx failed errno %d\n",(unsigned long)addr,errno);
        return NULL;
    }
    return p;
}
static uint32_t get32(uintptr_t where) {
    uint32_t x;memcpy(&x,(void*)where,4);return x;
}
static void put32(uintptr_t where,uint32_t x) {memcpy((void*)where,&x,4);}
static int checked_pointer(uintptr_t value) {return value>=BASE && value<BASE+LENGTH;}
static void stub_thumb(uintptr_t addr,unsigned ret) {
    /* movs r0,#ret; bx lr. Both 16-bit Thumb instructions. */
    const uint16_t ins[]={ (uint16_t)(0x2000u|(ret&255u)), 0x4770u };
    memcpy((void*)addr,ins,sizeof(ins));
}
static int init(const char* image) {
    FILE *f=fopen(image,"rb");if(!f)return 0;
    if(!map_at(BASE,IMAGE_MAP) || !map_at(HEAP,HEAP_MAP) || !map_at(USB_DATA,0x20000u)) {fclose(f);return 0;}
    size_t got=fread((void*)BASE,1,LENGTH,f);
    int next=fgetc(f);fclose(f);
    if(got!=LENGTH || next!=EOF) return 0;
    /* USB response never executes. The real compare/helper code remains intact. */
    stub_thumb(RESPONSE,0);
    /* All pages RX except vendor data/BSS page containing permission flag. */
    __builtin___clear_cache((char*)BASE,(char*)(BASE+IMAGE_MAP));
    if(mprotect((void*)BASE,IMAGE_MAP,PROT_READ|PROT_EXEC)!=0)return 0;
    if(mprotect((void*)0x0014d000u,4096,PROT_READ|PROT_WRITE)!=0)return 0;
    return 1;
}
static int replace_guest_stub(uintptr_t at,unsigned result) {
    uintptr_t page=at&~(uintptr_t)4095;
    if(mprotect((void*)page,4096,PROT_READ|PROT_WRITE)!=0)return 0;
    stub_thumb(at,result);
    __builtin___clear_cache((char*)page,(char*)(page+4096));
    return mprotect((void*)page,4096,PROT_READ|PROT_EXEC)==0;
}
static int cmp(const char *a,const char *b,unsigned n) {
    typedef int(*fun)(const char*,const char*,unsigned);
    return ((fun)COMPARE)(a,b,n);
}
static int permission(void) {return ((int(*)(void))PERMISSION)();}
static void oem(uintptr_t ctx){((void(*)(uintptr_t,uintptr_t))OEM)(0,ctx);}
static int classify(uintptr_t input){return ((int(*)(uintptr_t))CLASSIFY)(input);}
static void boot(void){((void(*)(void))BOOT)();}
static void usb_exit(void){((void(*)(void))USB_EXIT)();}
static void receive(uintptr_t context,uintptr_t packet){((void(*)(uintptr_t,uintptr_t))RECEIVE)(context,packet);}
static int result(const char *name,bool ok,const char *observed) {
    printf("{\"test\":\"%s\",\"pass\":%s,\"real_vendor_arm\":true,\"observation\":\"%s\"}\n",
        name,ok?"true":"false",observed);
    fflush(stdout);return ok?0:1;
}
static const char* real_oem_word(uintptr_t literal, unsigned len) {
    uint32_t p=get32(literal);
    if(!checked_pointer(p)||p+len>BASE+LENGTH)return NULL;
    return (const char*)(uintptr_t)p;
}
static void set_command(const char *command,size_t len) {
    /* The vendor OEM handler dereferences *param_2 + 4. */
    uint8_t *base=(uint8_t*)HEAP+0x400;
    memcpy(base,"oem ",4);
    memcpy(base+4,command,len);
    base[4+len]=0;
    put32(HEAP+0x100,(uint32_t)(uintptr_t)base);
}
int main(int argc,char**argv){
    if(argc!=3){fprintf(stderr,"usage: probe CASE private-verified-p3.raw\n");return 2;}
    alarm(6);
    const char *name=argv[1];
    if(!init(argv[2])){fprintf(stderr,"Cannot load private verified image\n");return 3;}
    if(get32(0x0012284c)!=FLAG||get32(0x00122928)!=FLAG)
        return result(name,false,"permission-or-writer-pointer-not-as-expected");
    volatile uint32_t *flag=(volatile uint32_t*)FLAG;
    if(!strncmp(name,"permission_",11)){
        unsigned value=0;
        if(!strcmp(name,"permission_1"))value=1;
        if(!strcmp(name,"permission_2"))value=2;
        if(!strcmp(name,"permission_highbit"))value=0x80000000u;
        *flag=value;
        int observed=permission();
        return result(name,observed==(value!=0) && *flag==value,
                      value==0?"zero-rejected":"nonzero-accepted");
    }
    if(!strncmp(name,"compare_",8)){
        char *a=(char*)HEAP+0x300;
        char *b=(char*)HEAP+0x400;
        strcpy(a,"abcdef");strcpy(b,"abcdef");
        unsigned n=6;bool want_equal=true;
        if(!strcmp(name,"compare_difference")){b[2]='z';want_equal=false;}
        else if(!strcmp(name,"compare_case")){b[0]='A';want_equal=false;}
        else if(!strcmp(name,"compare_zero_length")){n=0;}
        else if(!strcmp(name,"compare_short_length")){b[4]='z';n=4;}
        else if(!strcmp(name,"compare_early_nul")){a[1]=0;b[1]=0;}
        int actual=cmp(a,b,n);
        return result(name,(actual==0)==want_equal,"real-vendor-bounded-string-compare");
    }
    if(!strncmp(name,"oem_",4)){
        const char *token=real_oem_word(0x00122924u,37);
        const char *shell=real_oem_word(0x00122918u,6);
        const char *env=real_oem_word(0x00122930u,8);
        if(!token||!shell||!env)return result(name,false,"vendor-string-pointer-invalid");
        char b[256];const char *chosen=token;size_t length=37;
        if(!strncmp(name,"oem_mutate_",11)){
            unsigned pos=(unsigned)strtoul(name+11,NULL,10);
            if(pos>=37)return result(name,false,"invalid-mutated-byte-index");
            memcpy(b,token,37);
            b[pos]=(b[pos]=='x'?'y':'x');chosen=b;
        }else if(!strncmp(name,"oem_trunc_",10)){
            unsigned chars=(unsigned)strtoul(name+10,NULL,10);
            if(chars>37)return result(name,false,"invalid-truncation-length");
            memcpy(b,token,chars);chosen=b;length=chars;
        }else if(!strcmp(name,"oem_typo")){
            memcpy(b,token,37);b[36]=b[36]=='x'?'y':'x';chosen=b;
        }else if(!strcmp(name,"oem_short")){
            memcpy(b,token,36);chosen=b;length=36;
        }else if(!strcmp(name,"oem_suffix")){
            memcpy(b,token,37);b[37]='x';chosen=b;length=38;
        }else if(!strcmp(name,"oem_unrelated")){
            chosen="totally_invalid_command";length=strlen(chosen);
        }else if(!strcmp(name,"oem_shell_denied")){
            chosen=shell;length=6;
        }else if(!strcmp(name,"oem_env_denied")){
            chosen=env;length=8;
        }
        bool expect=!strcmp(name,"oem_exact")||!strcmp(name,"oem_suffix")||!strcmp(name,"oem_trunc_37");
        set_command(chosen,length);
        *flag=0;
        oem(HEAP+0x100);
        return result(name,(*flag!=0)==expect,
                      *flag?"authorization-flag-was-set":"authorization-flag-remained-zero");
    }
    if(!strncmp(name,"classify_",9)){
        uintptr_t payload=HEAP+0x800u;
        put32(payload,0xffffffffu);
        if(!strcmp(name,"classify_legacy")){
            put32(payload,get32(0x0010e3acu));
        }
        if(!strcmp(name,"classify_fdt")){
            if(!replace_guest_stub(FDT_CHECK&~1u,0))return 3;
        } else if(!strcmp(name,"classify_invalid")){
            if(!replace_guest_stub(FDT_CHECK&~1u,1))return 3;
        }
        int code=classify(payload);
        int expected=(!strcmp(name,"classify_legacy"))?1:
            (!strcmp(name,"classify_fdt"))?2:0;
        return result(name,code==expected,"actual-vendor-image-type-dispatch");
    }
    if(!strncmp(name,"receive_",8)){
        /* Exercise actual USB receive parser + actual in-memory copy only.
           Replace formatting/diagnostic/response functions with returns.
           Replace callback with return. Neither vendor USB nor peripherals run. */
        if(!replace_guest_stub(0x0012dbf4u,0) ||
           !replace_guest_stub(0x0012e96au,0))return 3;
        unsigned n=0, declared=0;
        if(!strcmp(name,"receive_one")){n=1;declared=1;}
        else if(!strcmp(name,"receive_31")){n=31;declared=31;}
        else if(!strcmp(name,"receive_256")){n=256;declared=256;}
        else if(!strcmp(name,"receive_4096")){n=4096;declared=4096;}
        else if(!strcmp(name,"receive_clip")){n=4200;declared=4096;}
        else return result(name,false,"unknown-receive-case");
        uintptr_t source=HEAP+0x2000u;
        for(unsigned i=0;i<n;i++)((volatile uint8_t*)source)[i]=(uint8_t)(i*7u+3u);
        memset((void*)(USB_DATA+0x800u),0,8192);
        put32(get32(0x00122ad4u),declared);
        put32(get32(0x00122ac4u),0);
        uintptr_t context=HEAP+0x400u, descriptor=HEAP+0x600u;
        uintptr_t vtable=HEAP+0x800u;
        put32(context+8,vtable);
        put32(vtable+0x10,(uint32_t)(RESPONSE|1u));
        put32(descriptor+0,source);
        put32(descriptor+8*4,0);
        put32(descriptor+9*4,n);
        receive(context,descriptor);
        bool copied=!memcmp((void*)(USB_DATA+0x800u),(void*)source,declared);
        bool count=get32(get32(0x00122ac4u))==declared;
        bool reset=get32(get32(0x00122ad4u))==0;
        bool cleared=get32(descriptor+9*4)==0;
        return result(name,copied&&count&&reset&&cleared,
            "real-vendor-transfer-limited-to-declared-size");
    }
    if(!strcmp(name,"boot_callback")){
        uint32_t global=get32(0x00122974u);
        if(!checked_pointer(global))return result(name,false,"callback-global-pointer-invalid");
        put32(global,HEAP+0x100);
        put32(HEAP+0x100+0x50,HEAP+0x200);
        put32(HEAP+0x200+0x10,0xdeadbeefu);
        boot();
        uint32_t observed=get32(HEAP+0x200+0x10);
        return result(name,observed==get32(0x00122978u),"callback-state-was-installed");
    }
    if(!strcmp(name,"usb_exit_state")){
        uint32_t ptr=get32(0x00121dd8u);
        if(!checked_pointer(ptr))return result(name,false,"usb-exit-global-pointer-invalid");
        *(volatile uint8_t*)(uintptr_t)ptr=0;
        usb_exit();
        return result(name,*(volatile uint8_t*)(uintptr_t)ptr==1,
                      "usb-exit-state-set-in-mock-ram");
    }
    return result(name,false,"unknown-case");
}