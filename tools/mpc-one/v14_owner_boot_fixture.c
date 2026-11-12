/* MPFree V14 owner boot-asset validation in isolated ARM QEMU.
 * This is original C code. It reads real owner /boot bytes from a virtual,
 * read-only ext4 fixture and NEVER executes or interprets vendor machine code.
 */
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
static uint32_t be32(const unsigned char *s){
    return (uint32_t)s[0]<<24 | (uint32_t)s[1]<<16 | (uint32_t)s[2]<<8 | s[3];
}
static uint32_t crc32(const unsigned char *s,size_t n){
    uint32_t c=~0u;
    for(size_t i=0;i<n;i++){c^=s[i];for(int j=0;j<8;j++)c=(c>>1)^((c&1)?0xedb88320u:0);}
    return ~c;
}
static unsigned char *slurp(const char *p,size_t *n){
    FILE *f=fopen(p,"rb");if(!f)return NULL;
    if(fseek(f,0,SEEK_END)){fclose(f);return NULL;}
    long m=ftell(f);if(m<0||m>2*1024*1024){fclose(f);return NULL;}
    rewind(f);unsigned char *b=malloc(m?(size_t)m:1);
    if(!b){fclose(f);return NULL;}
    size_t r=fread(b,1,(size_t)m,f);fclose(f);
    if(r!=(size_t)m){free(b);return NULL;}
    *n=r;return b;
}
static unsigned tests=0,failed=0;
static void check(const char *name,int ok){
    printf("{\"test\":\"%s\",\"pass\":%s}\n",name,ok?"true":"false");
    tests++;if(!ok)failed++;
}
static int uimage(const unsigned char *p,size_t n){
    if(n<64||!p||be32(p)!=0x27051956)return 0;
    unsigned char h[64];memcpy(h,p,64);uint32_t expect=be32(h+4);memset(h+4,0,4);
    uint32_t len=be32(p+12);
    return crc32(h,64)==expect&&len==n-64&&crc32(p+64,len)==be32(p+24);
}
static int fdt(const unsigned char *p,size_t n){
    if(n<40||!p||be32(p)!=0xd00dfeed)return 0;
    uint32_t total=be32(p+4),s=be32(p+8),t=be32(p+12);
    uint32_t ts=be32(p+32),ss=be32(p+36);
    return total>=40&&total<=n&&s<=total&&ss<=total-s&&t<=total&&ts<=total-t;
}
int main(int argc,char **argv){
    if(argc!=5)return 2;
    size_t ns=0,na=0,nc=0,nk=0;
    unsigned char *s=slurp(argv[1],&ns),*a=slurp(argv[2],&na),
                  *c=slurp(argv[3],&nc),*k=slurp(argv[4],&nk);
    check("real_script_loaded",s!=NULL);
    check("real_acva_dtb_loaded",a!=NULL);
    check("real_acvac_dtb_loaded",c!=NULL);
    check("real_kernel_header_loaded",k!=NULL);
    if(!s||!a||!c||!k){free(s);free(a);free(c);free(k);return 1;}
    check("owner_script_header_and_payload_crc",uimage(s,ns));
    check("owner_script_format",ns>72&&be32(s+64)<ns);
    check("owner_acva_fdt_structure",fdt(a,na));
    check("owner_acvac_fdt_structure",fdt(c,nc));
    check("distinct_device_tree_bytes",na!=nc||memcmp(a,c,na));
    check("owner_zimage_header_magic",nk>=0x28&&
       k[0x24]==0x18&&k[0x25]==0x28&&k[0x26]==0x6f&&k[0x27]==0x01);
    if(ns>64){
        unsigned char old=s[63];s[63]^=1;
        check("reject_script_header_mutation",!uimage(s,ns));s[63]=old;
        old=s[ns-1];s[ns-1]^=1;
        check("reject_script_payload_mutation",!uimage(s,ns));s[ns-1]=old;
        check("original_script_restored",uimage(s,ns));
    }
    if(na>40){
        unsigned char old=a[0];a[0]=0;
        check("reject_fdt_magic_corruption",!fdt(a,na));a[0]=old;
        old=a[4];a[4]=0x7f;
        check("reject_oversized_fdt_total",!fdt(a,na));a[4]=old;
        check("original_dtb_restored",fdt(a,na));
    }
    check("reject_short_script",!uimage(s,14));
    check("reject_short_fdt",!fdt(a,14));
    check("reject_null_script",!uimage(NULL,0));
    check("reject_null_fdt",!fdt(NULL,0));
    check("script_is_not_fit",ns>=4&&be32(s)!=0xd00dfeed);
    check("fdt_is_not_uimage",na>=4&&be32(a)!=0x27051956);
    printf("{\"summary\":\"owner-data-validation-not-vendor-code\",\"tests\":%u,\"failed\":%u}\n",tests,failed);
    puts(failed?"MPFREE_V14_FIXTURE_FAIL":"MPFREE_V14_FIXTURE_PASS");
    free(s);free(a);free(c);free(k);return failed?1:0;
}
