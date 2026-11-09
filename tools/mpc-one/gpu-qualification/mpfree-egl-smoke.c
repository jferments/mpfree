/*
 * MPFree offscreen EGL/OpenGL ES smoke test.
 * Uses GBM on the GPU render node; does not open /dev/dri/card* or modeset.
 * No EGL/GL development headers needed: ABI symbols are resolved at runtime.
 * Intended to compile for Linux armhf and run against the MPC's Mesa libraries.
 * MIT License. Copyright (c) 2026 MPFree contributors.
 */
#define _POSIX_C_SOURCE 200809L
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

typedef void *EGLDisplay;
typedef void *EGLConfig;
typedef void *EGLSurface;
typedef void *EGLContext;
typedef int32_t EGLint;
typedef uint32_t EGLenum;
typedef unsigned int GLenum;
typedef unsigned int GLbitfield;
typedef unsigned char GLubyte;

#define EGL_NONE 0x3038
#define EGL_EXTENSIONS 0x3055
#define EGL_VERSION 0x3054
#define EGL_VENDOR 0x3053
#define EGL_SURFACE_TYPE 0x3033
#define EGL_PBUFFER_BIT 0x0001
#define EGL_RENDERABLE_TYPE 0x3040
#define EGL_OPENGL_ES2_BIT 0x0004
#define EGL_RED_SIZE 0x3024
#define EGL_GREEN_SIZE 0x3023
#define EGL_BLUE_SIZE 0x3022
#define EGL_ALPHA_SIZE 0x3021
#define EGL_WIDTH 0x3057
#define EGL_HEIGHT 0x3056
#define EGL_CONTEXT_CLIENT_VERSION 0x3098
#define EGL_OPENGL_ES_API 0x30A0
#define EGL_PLATFORM_GBM_KHR 0x31D7
#define EGL_PLATFORM_SURFACELESS_MESA 0x31DD
#define GL_VENDOR 0x1F00
#define GL_RENDERER 0x1F01
#define GL_VERSION 0x1F02
#define GL_COLOR_BUFFER_BIT 0x00004000
#define GL_RGBA 0x1908
#define GL_UNSIGNED_BYTE 0x1401
#define GL_NO_ERROR 0

static EGLDisplay (*p_eglGetPlatformDisplayEXT)(EGLenum, void *, const EGLint *);
static void *(*p_eglGetProcAddress)(const char *);
static const char *(*p_eglQueryString)(EGLDisplay, EGLint);
static int (*p_eglInitialize)(EGLDisplay, EGLint *, EGLint *);
static int (*p_eglChooseConfig)(EGLDisplay, const EGLint *, EGLConfig *, EGLint, EGLint *);
static int (*p_eglBindAPI)(EGLenum);
static EGLContext (*p_eglCreateContext)(EGLDisplay, EGLConfig, EGLContext, const EGLint *);
static EGLSurface (*p_eglCreatePbufferSurface)(EGLDisplay, EGLConfig, const EGLint *);
static int (*p_eglMakeCurrent)(EGLDisplay, EGLSurface, EGLSurface, EGLContext);
static int (*p_eglDestroyContext)(EGLDisplay, EGLContext);
static int (*p_eglDestroySurface)(EGLDisplay, EGLSurface);
static int (*p_eglTerminate)(EGLDisplay);
static EGLint (*p_eglGetError)(void);
static const GLubyte *(*p_glGetString)(GLenum);
static void (*p_glViewport)(int, int, int, int);
static void (*p_glClearColor)(float, float, float, float);
static void (*p_glClear)(GLbitfield);
static void (*p_glFinish)(void);
static void (*p_glReadPixels)(int, int, int, int, GLenum, GLenum, void *);
static GLenum (*p_glGetError)(void);

static int contains_ci(const char *s, const char *needle) {
    if (!s || !needle) return 0;
    size_t n = strlen(needle);
    for (; *s; ++s) {
        size_t i = 0;
        for (; i < n; ++i) {
            unsigned char a = (unsigned char)s[i];
            unsigned char b = (unsigned char)needle[i];
            if (!a) break;
            if (a >= 'A' && a <= 'Z') a += 32;
            if (b >= 'A' && b <= 'Z') b += 32;
            if (a != b) break;
        }
        if (i == n) return 1;
    }
    return 0;
}

#define REQUIRE(lib, name) do {                                              \
    *(void **)(&p_##name) = dlsym((lib), #name);                              \
    if (!p_##name) {                                                         \
        fprintf(stderr, "Missing symbol %s: %s\n", #name, dlerror());       \
        return 10;                                                           \
    }                                                                        \
} while (0)

static int egl_render(EGLDisplay display, const char *platform) {
    EGLint major = 0, minor = 0;
    if (!display) {
        fprintf(stderr, "%s: EGL_NO_DISPLAY (error 0x%x)\n", platform, (unsigned)p_eglGetError());
        return 1;
    }
    if (!p_eglInitialize(display, &major, &minor)) {
        fprintf(stderr, "%s: eglInitialize failed (0x%x)\n", platform, (unsigned)p_eglGetError());
        return 1;
    }
    printf("EGL platform: %s\nEGL version: %d.%d\nEGL vendor: %s\n",
           platform, major, minor, p_eglQueryString(display, EGL_VENDOR));
    const char *display_ext = p_eglQueryString(display, EGL_EXTENSIONS);
    printf("EGL surfaceless-context extension: %s\n",
           display_ext && strstr(display_ext, "EGL_KHR_surfaceless_context") ? "yes" : "no");

    EGLConfig config = NULL;
    EGLint count = 0;
    const EGLint rgba[] = {
        EGL_SURFACE_TYPE, EGL_PBUFFER_BIT,
        EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
        EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
        EGL_NONE
    };
    const EGLint rgb565[] = {
        EGL_SURFACE_TYPE, EGL_PBUFFER_BIT,
        EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
        EGL_RED_SIZE, 5, EGL_GREEN_SIZE, 6, EGL_BLUE_SIZE, 5,
        EGL_NONE
    };
    if (!p_eglChooseConfig(display, rgba, &config, 1, &count) || count == 0) {
        config = NULL;
        count = 0;
        if (!p_eglChooseConfig(display, rgb565, &config, 1, &count) || count == 0) {
            fprintf(stderr, "No GLES2 pbuffer EGLConfig (0x%x)\n", (unsigned)p_eglGetError());
            p_eglTerminate(display);
            return 1;
        }
    }
    if (!p_eglBindAPI(EGL_OPENGL_ES_API)) {
        fprintf(stderr, "eglBindAPI failed (0x%x)\n", (unsigned)p_eglGetError());
        p_eglTerminate(display);
        return 1;
    }
    const EGLint ctx_attr[] = {EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE};
    const EGLint surf_attr[] = {EGL_WIDTH, 256, EGL_HEIGHT, 256, EGL_NONE};
    EGLContext ctx = p_eglCreateContext(display, config, NULL, ctx_attr);
    if (!ctx) {
        fprintf(stderr, "eglCreateContext failed (0x%x)\n", (unsigned)p_eglGetError());
        p_eglTerminate(display);
        return 1;
    }
    EGLSurface surf = p_eglCreatePbufferSurface(display, config, surf_attr);
    if (!surf) {
        fprintf(stderr, "eglCreatePbufferSurface failed (0x%x)\n", (unsigned)p_eglGetError());
        p_eglDestroyContext(display, ctx);
        p_eglTerminate(display);
        return 1;
    }
    if (!p_eglMakeCurrent(display, surf, surf, ctx)) {
        fprintf(stderr, "eglMakeCurrent failed (0x%x)\n", (unsigned)p_eglGetError());
        p_eglDestroySurface(display, surf);
        p_eglDestroyContext(display, ctx);
        p_eglTerminate(display);
        return 1;
    }

    const char *vendor = (const char *)p_glGetString(GL_VENDOR);
    const char *renderer = (const char *)p_glGetString(GL_RENDERER);
    const char *version = (const char *)p_glGetString(GL_VERSION);
    printf("GL_VENDOR: %s\nGL_RENDERER: %s\nGL_VERSION: %s\n",
           vendor ? vendor : "(null)", renderer ? renderer : "(null)",
           version ? version : "(null)");

    p_glViewport(0, 0, 256, 256);
    p_glClearColor(0.2f, 0.6f, 0.8f, 1.0f);
    p_glClear(GL_COLOR_BUFFER_BIT);
    p_glFinish();
    unsigned char rgba_px[4] = {0, 0, 0, 0};
    p_glReadPixels(128, 128, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, rgba_px);
    GLenum gl_error = p_glGetError();
    printf("Readback RGBA: %u %u %u %u (expected approximately 51 153 204 255)\n",
           rgba_px[0], rgba_px[1], rgba_px[2], rgba_px[3]);
    printf("glGetError: 0x%x\n", gl_error);
    int pixel_ok = gl_error == GL_NO_ERROR &&
        abs((int)rgba_px[0] - 51) <= 12 &&
        abs((int)rgba_px[1] - 153) <= 12 &&
        abs((int)rgba_px[2] - 204) <= 12;
    int software = contains_ci(renderer, "llvmpipe") || contains_ci(renderer, "softpipe") ||
                   contains_ci(renderer, "swrast");
    int hardware = contains_ci(renderer, "panfrost") || contains_ci(renderer, "mali");
    printf("Pixel validation: %s\n", pixel_ok ? "PASS" : "FAIL");
    printf("Renderer classification: %s\n",
           software ? "SOFTWARE" : hardware ? "PANFROST/MALI" : "UNKNOWN (inspect GL_RENDERER)");
    p_eglMakeCurrent(display, NULL, NULL, NULL);
    p_eglDestroySurface(display, surf);
    p_eglDestroyContext(display, ctx);
    p_eglTerminate(display);
    if (!pixel_ok) return 3;
    if (software) return 4;
    if (!hardware) return 5;
    return 0;
}

int main(void) {
    puts("=== MPFree EGL/GLES2 offscreen render smoke test ===");
    puts("NO display modesetting; 256x256 offscreen buffer; no GUI window.");
    fflush(stdout);

    void *egl = dlopen("libEGL.so.1", RTLD_NOW | RTLD_LOCAL);
    void *gles = dlopen("libGLESv2.so.2", RTLD_NOW | RTLD_LOCAL);
    void *gbm = dlopen("libgbm.so.1", RTLD_NOW | RTLD_LOCAL);
    if (!egl || !gles) {
        fprintf(stderr, "Missing graphics library: EGL=%s GLES=%s\n",
                egl ? "present" : "missing", gles ? "present" : "missing");
        return 10;
    }
    REQUIRE(egl, eglGetProcAddress);
    REQUIRE(egl, eglQueryString);
    REQUIRE(egl, eglInitialize);
    REQUIRE(egl, eglChooseConfig);
    REQUIRE(egl, eglBindAPI);
    REQUIRE(egl, eglCreateContext);
    REQUIRE(egl, eglCreatePbufferSurface);
    REQUIRE(egl, eglMakeCurrent);
    REQUIRE(egl, eglDestroyContext);
    REQUIRE(egl, eglDestroySurface);
    REQUIRE(egl, eglTerminate);
    REQUIRE(egl, eglGetError);
    REQUIRE(gles, glGetString);
    REQUIRE(gles, glViewport);
    REQUIRE(gles, glClearColor);
    REQUIRE(gles, glClear);
    REQUIRE(gles, glFinish);
    REQUIRE(gles, glReadPixels);
    REQUIRE(gles, glGetError);

    *(void **)(&p_eglGetPlatformDisplayEXT) = p_eglGetProcAddress("eglGetPlatformDisplayEXT");
    if (!p_eglGetPlatformDisplayEXT) {
        puts("EGL platform-base entrypoint unavailable: unable to use explicit offscreen platform safely.");
        return 11;
    }
    const char *ext = p_eglQueryString(NULL, EGL_EXTENSIONS);
    printf("EGL client platform extensions: %s\n", ext ? ext : "(unavailable)");

    /* Preferred path: explicit render node GBM device, never a DRM master/display card. */
    if (gbm) {
        int fd = open("/dev/dri/renderD128", O_RDWR | O_CLOEXEC);
        if (fd < 0) {
            fprintf(stderr, "Cannot open /dev/dri/renderD128: %s\n", strerror(errno));
        } else {
            void *(*gbm_create_device)(int) = NULL;
            void (*gbm_device_destroy)(void *) = NULL;
            *(void **)(&gbm_create_device) = dlsym(gbm, "gbm_create_device");
            *(void **)(&gbm_device_destroy) = dlsym(gbm, "gbm_device_destroy");
            if (gbm_create_device && gbm_device_destroy) {
                void *device = gbm_create_device(fd);
                if (device) {
                    puts("GBM render-node device created successfully.");
                    EGLDisplay display = p_eglGetPlatformDisplayEXT(EGL_PLATFORM_GBM_KHR, device, NULL);
                    int result = egl_render(display, "GBM renderD128");
                    gbm_device_destroy(device);
                    close(fd);
                    if (result == 0) {
                        puts("RESULT: PASS — Panfrost/Mali offscreen GPU rendering works.");
                        return 0;
                    }
                    printf("GBM attempt result: %d; surfaceless fallback is diagnostic only.\n", result);
                } else {
                    puts("gbm_create_device failed.");
                    close(fd);
                }
            } else {
                puts("GBM functions unavailable.");
                close(fd);
            }
        }
    } else {
        puts("libgbm.so.1 unavailable.");
    }
    puts("Trying surfaceless EGL only for diagnostic information (may use software rendering).");
    EGLDisplay display = p_eglGetPlatformDisplayEXT(EGL_PLATFORM_SURFACELESS_MESA, NULL, NULL);
    int result = egl_render(display, "Mesa surfaceless");
    if (result == 0) {
        puts("RESULT: PASS — Panfrost/Mali offscreen rendering (surfaceless path).");
        return 0;
    }
    printf("RESULT: NOT QUALIFIED (code %d). Offscreen acceleration is not yet demonstrated.\n", result);
    return result ? result : 1;
}