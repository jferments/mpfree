/* MPFree independent USB endpoint-descriptor discovery test model.
 * NOT AKAI code and never touches physical USB/flash/eMMC. Derived from
 * an owner's offline static inspection of libaz01-prog endpoint scanning.
 * Strict defensive bounds do not imply identical vendor behavior.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

struct endpoints {
    uint8_t in_addr, out_addr;
};

/* Parse an embedded USB interface and its following endpoint descriptors.
 * This model records the first IN and first OUT endpoint addresses, using
 * bEndpointAddress bit 7. Other endpoint attributes are not classified.
 */
static int discover_endpoints(const uint8_t *buf, size_t bytes,
                              size_t interface_offset, struct endpoints *out) {
    size_t declared, off;
    unsigned found = 0, counted = 0;
    if (!buf || !out || bytes < 9 || buf[0] < 9 || buf[1] != 2)
        return 0;
    declared = (size_t)buf[2] | ((size_t)buf[3] << 8);
    if (declared < buf[0] || declared > bytes || interface_offset > declared)
        return 0;
    if (declared - interface_offset < 9 || buf[interface_offset] < 9 ||
        buf[interface_offset + 1] != 4 ||
        buf[interface_offset] > declared - interface_offset)
        return 0;
    memset(out, 0, sizeof *out);
    off = interface_offset + buf[interface_offset];
    while (off + 2 <= declared && counted < buf[interface_offset + 4]) {
        size_t length = buf[off];
        unsigned type = buf[off + 1];
        if (length < 2 || length > declared - off)
            return 0;
        if (type == 4) break;  /* Do not conflate adjacent interfaces. */
        if (type == 5) {
            uint8_t address;
            if (length < 7) return 0;
            address = buf[off + 2];
            ++counted;
            if ((address & 0x80) && !(found & 1)) {
                out->in_addr = address;
                found |= 1;
            } else if (!(address & 0x80) && !(found & 2)) {
                out->out_addr = address;
                found |= 2;
            }
            if (found == 3) return 1;
        }
        off += length;
    }
    return found == 3;
}

struct case_spec { const char *name; int mutation, ok; uint8_t in, out; };
static const struct case_spec cases[] = {
    {"endpoint_in_then_out",0,1,0x81,0x02},
    {"endpoint_out_then_in",1,1,0x81,0x02},
    {"endpoint_in_only",2,0,0x81,0},
    {"endpoint_out_only",3,0,0,0x02},
    {"endpoint_none_declared",4,0,0,0},
    {"endpoint_duplicate_in",5,0,0x81,0},
    {"endpoint_duplicate_out",6,0,0,0x01},
    {"endpoint_short_length",7,0,0,0},
    {"endpoint_zero_length",8,0,0,0},
    {"endpoint_not_endpoint_type",9,0,0,0x02},
    {"endpoint_truncated_buffer",10,0,0,0},
    {"endpoint_declared_too_long",11,0,0,0},
    {"endpoint_wrong_interface_type",12,0,0,0},
    {"endpoint_short_interface",13,0,0,0},
    {"endpoint_high_numbers",14,1,0x8e,0x0f},
    {"endpoint_count_one",15,0,0x81,0},
    {"endpoint_interposed_descriptor",16,1,0x81,0x02},
    {"endpoint_invalid_onebyte_length",17,0,0,0},
    {"endpoint_nonbulk_attributes_accepted",18,1,0x81,0x02},
    {"endpoint_duplicate_before_out",19,1,0x81,0x02},
    {"endpoint_adjacent_interface_stop",20,0,0x81,0}
};

int main(void) {
    unsigned passed = 0, n = (unsigned)(sizeof cases / sizeof *cases);
    unsigned i;
    for (i = 0; i < n; i++) {
        const struct case_spec *t = &cases[i];
        uint8_t b[48] = {
            9,2,32,0,1,1,0,0x80,50,  /* config */
            9,4,0,0,2,0xff,0x42,0x03,0, /* interface */
            7,5,0x81,2,64,0,0,        /* endpoint IN */
            7,5,0x02,2,64,0,0         /* endpoint OUT */
        };
        size_t size = 32;
        struct endpoints e = {0,0};
        int got, ok;
        switch (t->mutation) {
        case 1: b[20]=0x02; b[27]=0x81; break;
        case 2: b[13]=1; b[2]=25; size=25; break;
        case 3: b[13]=1; b[20]=0x02; b[2]=25; size=25; break;
        case 4: b[13]=0; break;
        case 5: b[27]=0x82; break;
        case 6: b[20]=0x01; break;
        case 7: b[18]=6; break;
        case 8: b[18]=0; break;
        case 9: b[19]=0x24; break;
        case 10: size=25; break;
        case 11: b[2]=100; break;
        case 12: b[10]=5; break;
        case 13: b[9]=8; break;
        case 14: b[20]=0x8e; b[27]=0x0f; break;
        case 15: b[13]=1; break;
        case 16:
            memmove(b + 20, b + 18, 14);
            b[18]=2; b[19]=0x24; b[2]=34; size=34; break;
        case 17: b[18]=1; break;
        case 18: b[21]=3; b[28]=3; break;
        case 19:
            memmove(b + 32, b + 25, 7);
            b[25]=7; b[26]=5; b[27]=0x82; b[28]=2;
            b[29]=64; b[30]=0; b[31]=0;
            b[2]=39; b[13]=3; size=39; break;
        case 20: b[26]=4; break;
        default: break;
        }
        got=discover_endpoints(b,size,9,&e);
        ok = (got == t->ok) && (e.in_addr == t->in) &&
             (e.out_addr == t->out);
        passed += (unsigned)ok;
        printf("{\"test\":\"%s\",\"expected\":%s,\"actual\":%s,\"in\":%u,\"out\":%u,\"pass\":%s}\n",
               t->name,t->ok?"true":"false",got?"true":"false",
               (unsigned)e.in_addr,(unsigned)e.out_addr,ok?"true":"false");
    }
    printf("MPFREE_USB_ENDPOINT_MODEL_TESTS %u/%u\n",passed,n);
    puts(passed == n ? "MPFREE_USB_ENDPOINT_MODEL_PASS" : "MPFREE_USB_ENDPOINT_MODEL_FAIL");
    return passed == n ? 0 : 1;
}
