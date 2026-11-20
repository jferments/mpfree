/* MPFree synthetic USB interface classifier.
 *
 * Behavioral model derived from an owner's private, static Ghidra analysis
 * of libaz01-prog's 0x1d9fc USB interface selection code. This is NOT
 * proprietary AKAI code. No USB or block-device operations are performed.
 * Output is one JSON line per synthetic test case plus a final result.
 */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

enum backend { BACKEND_NONE = 0, BACKEND_A, BACKEND_B, BACKEND_C };

static const char *backend_name(enum backend id) {
    switch (id) {
    case BACKEND_A: return "candidate_0x12a74";
    case BACKEND_B: return "candidate_0x12b48";
    case BACKEND_C: return "candidate_0x12bd8";
    default: return "none";
    }
}

/* USB config+interface descriptor walk, with strict bounds.
   Unlike a decompiler rendering, malformed lengths are rejected on
   purpose; do not assume the vendor implementation has these guards. */
static enum backend select_backend(const unsigned char *buf, size_t size) {
    size_t off;
    size_t declared;
    unsigned max_interfaces, seen;
    if (!buf || size < 9 || buf[0] < 9 || buf[1] != 2)
        return BACKEND_NONE;
    declared = (size_t)buf[2] | ((size_t)buf[3] << 8);
    if (declared < buf[0] || declared > size)
        return BACKEND_NONE;
    off = buf[0];
    seen = 0;
    max_interfaces = buf[4];
    while (off + 2 <= declared && seen < max_interfaces) {
        unsigned step = buf[off];
        if (step < 2 || step > declared - off)
            return BACKEND_NONE;
        if (buf[off + 1] == 4) {
            ++seen;
            if (step >= 9 && buf[off + 5] == 0xff) {
                unsigned sub = buf[off + 6];
                unsigned proto = buf[off + 7];
                if (sub == 0x42 && proto == 0x03) return BACKEND_A;
                if (sub == 0x61 && proto == 0x7a) return BACKEND_B;
                if (sub == 0x06 && proto == 0x05) return BACKEND_C;
            }
        }
        off += step;
    }
    return BACKEND_NONE;
}

struct test_case {
    const char *name;
    unsigned char subclass, protocol, interface_class;
    int variant;
    enum backend expected;
};

/* 0 normal, 1 truncated, 2 wrong config type, 3 malformed 0 length,
   4 interface descriptor length 8, 5 invalid total length,
   6 non-interface descriptor type, 7 no interfaces declared. */
static const struct test_case cases[] = {
    {"usb_backend_42_03",0x42,0x03,0xff,0,BACKEND_A},
    {"usb_backend_61_7a",0x61,0x7a,0xff,0,BACKEND_B},
    {"usb_backend_06_05",0x06,0x05,0xff,0,BACKEND_C},
    {"class_mismatch",0x42,0x03,0x08,0,BACKEND_NONE},
    {"subclass_mismatch",0x43,0x03,0xff,0,BACKEND_NONE},
    {"protocol_mismatch",0x42,0x04,0xff,0,BACKEND_NONE},
    {"truncated_descriptor",0x42,0x03,0xff,1,BACKEND_NONE},
    {"wrong_config_type",0x42,0x03,0xff,2,BACKEND_NONE},
    {"zero_length_descriptor",0x42,0x03,0xff,3,BACKEND_NONE},
    {"short_interface_descriptor",0x42,0x03,0xff,4,BACKEND_NONE},
    {"invalid_total_length",0x42,0x03,0xff,5,BACKEND_NONE},
    {"not_an_interface",0x42,0x03,0xff,6,BACKEND_NONE},
    {"zero_interfaces_declared",0x42,0x03,0xff,7,BACKEND_NONE},
};

int main(void) {
    unsigned passed = 0;
    unsigned total = (unsigned)(sizeof(cases) / sizeof(cases[0]));
    unsigned idx;
    for (idx = 0; idx < total; ++idx) {
        const struct test_case *test = &cases[idx];
        unsigned char b[18] = {
            9,2,18,0,1,1,0,0x80,50,
            9,4,0,0,2,0xff,0x42,0x03,0
        };
        size_t size = sizeof b;
        enum backend got;
        int ok;
        b[14] = test->interface_class;
        b[15] = test->subclass;
        b[16] = test->protocol;
        switch (test->variant) {
        case 1: size=16; break;
        case 2: b[1]=3; break;
        case 3: b[9]=0; break;
        case 4: b[9]=8; break;
        case 5: b[2]=100; break;
        case 6: b[10]=5; break;
        case 7: b[4]=0; break;
        default: break;
        }
        got=select_backend(b,size);
        ok=(got == test->expected);
        passed += (unsigned)ok;
        printf("{\"test\":\"%s\",\"expected\":\"%s\",\"actual\":\"%s\",\"pass\":%s}\n",
               test->name,backend_name(test->expected),
               backend_name(got),ok?"true":"false");
    }
    printf("MPFREE_USB_DESCRIPTOR_MODEL_TESTS %u/%u\n",passed,total);
    if (passed == total) puts("MPFREE_USB_DESCRIPTOR_MODEL_PASS");
    else puts("MPFREE_USB_DESCRIPTOR_MODEL_FAIL");
    return passed == total ? 0 : 1;
}
