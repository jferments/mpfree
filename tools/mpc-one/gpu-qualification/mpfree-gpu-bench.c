/*
 * MPFree GPU qualification: offscreen ES2 composite UI workload.
 * MIT License — Copyright (c) 2026 MPFree contributors.
 * No /dev/dri/card*, no KMS master, no framebuffer writes, no audio access.
 * Uses the proven Mesa surfaceless EGL platform + an RGBA8 FBO.
 * Builds without EGL/GLES development headers: runtime symbol loading.
 */
#define _POSIX_C_SOURCE 200809L
#include <dlfcn.h>
#include <errno.h>
#include <math.h>
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <sys/resource.h>

#define WIDTH 800
#define HEIGHT 480
#define WARMUP 20
#define SAMPLES 120
#define PI 3.14159265358979323846f

typedef void *EGLDisplay;
typedef void *EGLConfig;
typedef void *EGLSurface;
typedef void *EGLContext;
typedef int32_t EGLint;
typedef uint32_t EGLenum;
typedef unsigned int GLenum;
typedef unsigned int GLuint;
typedef int GLint;
typedef unsigned int GLbitfield;
typedef int GLsizei;
typedef ptrdiff_t GLsizeiptr;
typedef char GLchar;
typedef unsigned char GLboolean;
typedef unsigned char GLubyte;
#define EGL_EXTENSIONS 0x3055
#define EGL_VENDOR 0x3053
#define EGL_NONE 0x3038
#define EGL_SURFACE_TYPE 0x3033
#define EGL_PBUFFER_BIT 0x0001
#define EGL_RENDERABLE_TYPE 0x3040
#define EGL_OPENGL_ES2_BIT 0x0004
#define EGL_RED_SIZE 0x3024
#define EGL_GREEN_SIZE 0x3023
#define EGL_BLUE_SIZE 0x3022
#define EGL_ALPHA_SIZE 0x3021
#define EGL_CONTEXT_CLIENT_VERSION 0x3098
#define EGL_OPENGL_ES_API 0x30A0
#define EGL_PLATFORM_SURFACELESS_MESA 0x31DD
#define GL_VENDOR 0x1F00
#define GL_RENDERER 0x1F01
#define GL_VERSION 0x1F02
#define GL_ARRAY_BUFFER 0x8892
#define GL_DYNAMIC_DRAW 0x88E8
#define GL_FLOAT 0x1406
#define GL_FALSE 0
#define GL_VERTEX_SHADER 0x8B31
#define GL_FRAGMENT_SHADER 0x8B30
#define GL_COMPILE_STATUS 0x8B81
#define GL_LINK_STATUS 0x8B82
#define GL_TRIANGLES 0x0004
#define GL_LINE_STRIP 0x0003
#define GL_COLOR_BUFFER_BIT 0x00004000
#define GL_FRAMEBUFFER 0x8D40
#define GL_COLOR_ATTACHMENT0 0x8CE0
#define GL_FRAMEBUFFER_COMPLETE 0x8CD5
#define GL_TEXTURE_2D 0x0DE1
#define GL_TEXTURE_MIN_FILTER 0x2801
#define GL_TEXTURE_MAG_FILTER 0x2800
#define GL_LINEAR 0x2601
#define GL_RGBA 0x1908
#define GL_UNSIGNED_BYTE 0x1401
#define GL_NO_ERROR 0

static EGLDisplay (*eglGetPlatformDisplayEXT)(EGLenum, void *, const EGLint *);
static const char *(*eglQueryString)(EGLDisplay, EGLint);
static int (*eglInitialize)(EGLDisplay, EGLint *, EGLint *);
static int (*eglChooseConfig)(EGLDisplay, const EGLint *, EGLConfig *, EGLint, EGLint *);
static int (*eglBindAPI)(EGLenum);
static EGLContext (*eglCreateContext)(EGLDisplay, EGLConfig, EGLContext, const EGLint *);
static int (*eglMakeCurrent)(EGLDisplay, EGLSurface, EGLSurface, EGLContext);
static void *(*eglGetProcAddress)(const char *);
static int (*eglDestroyContext)(EGLDisplay, EGLContext);
static int (*eglTerminate)(EGLDisplay);
static EGLint (*eglGetError)(void);
static const GLubyte *(*glGetString)(GLenum);
static void (*glViewport)(GLint, GLint, GLsizei, GLsizei);
static void (*glClearColor)(float, float, float, float);
static void (*glClear)(GLbitfield);
static void (*glFinish)(void);
static GLenum (*glGetError)(void);
static void (*glReadPixels)(GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, void *);
static void (*glGenFramebuffers)(GLsizei, GLuint *);
static void (*glBindFramebuffer)(GLenum, GLuint);
static void (*glFramebufferTexture2D)(GLenum, GLenum, GLenum, GLuint, GLint);
static GLenum (*glCheckFramebufferStatus)(GLenum);
static void (*glGenTextures)(GLsizei, GLuint *);
static void (*glBindTexture)(GLenum, GLuint);
static void (*glTexImage2D)(GLenum, GLint, GLint, GLsizei, GLsizei, GLint, GLenum, GLenum, const void *);
static void (*glTexParameteri)(GLenum, GLenum, GLint);
static void (*glGenBuffers)(GLsizei, GLuint *);
static void (*glBindBuffer)(GLenum, GLuint);
static void (*glBufferData)(GLenum, GLsizeiptr, const void *, GLenum);
static void (*glEnableVertexAttribArray)(GLuint);
static void (*glVertexAttribPointer)(GLuint, GLint, GLenum, GLboolean, GLsizei, const void *);
static GLuint (*glCreateShader)(GLenum);
static void (*glShaderSource)(GLuint, GLsizei, const GLchar *const *, const GLint *);
static void (*glCompileShader)(GLuint);
static void (*glGetShaderiv)(GLuint, GLenum, GLint *);
static GLuint (*glCreateProgram)(void);
static void (*glAttachShader)(GLuint, GLuint);
static void (*glBindAttribLocation)(GLuint, GLuint, const GLchar *);
static void (*glLinkProgram)(GLuint);
static void (*glGetProgramiv)(GLuint, GLenum, GLint *);
static void (*glUseProgram)(GLuint);
static GLint (*glGetUniformLocation)(GLuint, const GLchar *);
static void (*glUniform4f)(GLint, float, float, float, float);
static void (*glDrawArrays)(GLenum, GLint, GLsizei);
static void (*glDeleteTextures)(GLsizei, const GLuint *);
static void (*glDeleteFramebuffers)(GLsizei, const GLuint *);
static void (*glDeleteBuffers)(GLsizei, const GLuint *);
static void (*glDeleteProgram)(GLuint);
static void (*glDeleteShader)(GLuint);

#define LOAD(LIB, NAME) do { *(void **)(&NAME) = dlsym((LIB), #NAME); \
    if (!(NAME)) { fprintf(stderr, "Missing %s: %s\n", #NAME, dlerror()); return 10; } \
} while (0)
static double mono_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec * 1000.0 + (double)ts.tv_nsec / 1000000.0;
}
static int cmp_double(const void *a, const void *b) {
    const double x = *(const double *)a, y = *(const double *)b;
    return (x > y) - (x < y);
}
static float vertices[8192];
static size_t nverts;
static void add_pt(float x, float y) {
    if (nverts >= sizeof(vertices) / sizeof(vertices[0]) / 2) {
        fputs("Internal vertex array overflow\n", stderr);
        exit(13);
    }
    vertices[nverts * 2] = 2.0f * x / WIDTH - 1.0f;
    vertices[nverts * 2 + 1] = 2.0f * y / HEIGHT - 1.0f;
    nverts++;
}
static void triangle(float x0, float y0, float x1, float y1, float x2, float y2) {
    add_pt(x0,y0); add_pt(x1,y1); add_pt(x2,y2);
}
static void rect(float x0, float y0, float x1, float y1) {
    triangle(x0,y0,x1,y0,x1,y1);
    triangle(x0,y0,x1,y1,x0,y1);
}
static GLint ucolor;
static void draw(GLenum prim, float r, float g, float b, float a) {
    glBufferData(GL_ARRAY_BUFFER, (GLsizeiptr)(nverts * 2 * sizeof(float)), vertices, GL_DYNAMIC_DRAW);
    glUniform4f(ucolor, r,g,b,a);
    glDrawArrays(prim, 0, (GLsizei)nverts);
    nverts = 0;
}
static void scene(int frame) {
    const float t = frame * 0.10f;
    glClearColor(0.035f,0.049f,0.082f,1.f);
    glClear(GL_COLOR_BUFFER_BIT);
    /* Left: four animated 256-point audio waveforms. */
    for (int track = 0; track < 4; ++track) {
        nverts = 0;
        const float base = 435.0f - track * 95.0f;
        const float amp = 24.0f + 3.0f * track;
        for (int i=0; i<256; ++i) {
            float x = 20.0f + i * 2.04f;
            float v = 0.64f * sinf(i*0.089f*(1+track*.13f)+t) +
                      0.36f * sinf(i*0.223f+t*1.7f);
            add_pt(x, base + amp*v);
        }
        draw(GL_LINE_STRIP, .15f+.15f*track, .83f-.10f*track, .78f+.05f*track, 1.f);
    }
    /* Right-top: 64 changing frequency-spectrum bars (one batch). */
    nverts = 0;
    for (int i=0; i<64; ++i) {
        float env = expf(-i/22.f);
        float h = 15.f + 125.f * env * fabsf(sinf(i*.17f+t*.57f) * cosf(i*.09f+t*.42f));
        float x = 561.f + i*3.35f;
        rect(x, 294.f-h, x+2.35f, 294.f);
    }
    draw(GL_TRIANGLES, .40f, .67f, 1.f, 1.f);
    /* Right-bottom: 16x8 sequencer steps. */
    nverts = 0;
    for (int row=0; row<8; ++row) {
        for (int col=0; col<16; ++col) {
            float x = 557.f + col*13.3f;
            float y = 38.f + row*12.2f;
            rect(x,y,x+10.9f,y+9.4f);
        }
    }
    draw(GL_TRIANGLES, .17f, .24f, .34f, 1.f);
    nverts = 0;
    for (int row=0; row<8; ++row) {
        for (int col=0; col<16; ++col) {
            if ((row*7+col*11+frame/7)%13 < 4) {
                float x = 557.f + col*13.3f;
                float y = 38.f + row*12.2f;
                rect(x,y,x+10.9f,y+9.4f);
            }
        }
    }
    draw(GL_TRIANGLES, .93f, .46f, .27f, 1.f);
}
static int snapshot(const char *path) {
    const size_t size = (size_t)WIDTH*HEIGHT*4;
    unsigned char *rgba = malloc(size);
    if (!rgba) return 11;
    glReadPixels(0,0,WIDTH,HEIGHT,GL_RGBA,GL_UNSIGNED_BYTE,rgba);
    if (glGetError()!=GL_NO_ERROR) { free(rgba); return 12; }
    FILE *f = fopen(path,"wb");
    if (!f) { perror("fopen snapshot"); free(rgba); return 12; }
    fprintf(f, "P6\n%d %d\n255\n", WIDTH, HEIGHT);
    for (int y=HEIGHT-1; y>=0; --y) {
        for (int x=0; x<WIDTH; ++x) {
            const unsigned char *p = &rgba[((size_t)y*WIDTH+x)*4];
            if (fwrite(p,1,3,f)!=3) { fclose(f); free(rgba); return 12; }
        }
    }
    if (fclose(f)!=0) { free(rgba); return 12; }
    free(rgba);
    return 0;
}
static int load_all(void) {
    void *egl = dlopen("libEGL.so.1", RTLD_NOW|RTLD_LOCAL);
    void *gles = dlopen("libGLESv2.so.2", RTLD_NOW|RTLD_LOCAL);
    if (!egl || !gles) { fputs("Missing EGL or GLES runtime\n",stderr); return 10; }
    LOAD(egl, eglGetProcAddress);
    LOAD(egl, eglQueryString);
    LOAD(egl, eglInitialize);
    LOAD(egl, eglChooseConfig);
    LOAD(egl, eglBindAPI);
    LOAD(egl, eglCreateContext);
    LOAD(egl, eglMakeCurrent);
    LOAD(egl, eglDestroyContext);
    LOAD(egl, eglTerminate);
    LOAD(egl, eglGetError);
    *(void **)(&eglGetPlatformDisplayEXT) = eglGetProcAddress("eglGetPlatformDisplayEXT");
    if (!eglGetPlatformDisplayEXT) { fputs("Missing eglGetPlatformDisplayEXT\n",stderr); return 10; }
#define GLLOAD(NAME) LOAD(gles, NAME)
    GLLOAD(glGetString); GLLOAD(glViewport); GLLOAD(glClearColor); GLLOAD(glClear);
    GLLOAD(glFinish); GLLOAD(glGetError); GLLOAD(glReadPixels);
    GLLOAD(glGenFramebuffers); GLLOAD(glBindFramebuffer); GLLOAD(glFramebufferTexture2D);
    GLLOAD(glCheckFramebufferStatus); GLLOAD(glGenTextures); GLLOAD(glBindTexture);
    GLLOAD(glTexImage2D); GLLOAD(glTexParameteri); GLLOAD(glGenBuffers);
    GLLOAD(glBindBuffer); GLLOAD(glBufferData); GLLOAD(glEnableVertexAttribArray);
    GLLOAD(glVertexAttribPointer); GLLOAD(glCreateShader); GLLOAD(glShaderSource);
    GLLOAD(glCompileShader); GLLOAD(glGetShaderiv); GLLOAD(glCreateProgram);
    GLLOAD(glAttachShader); GLLOAD(glBindAttribLocation); GLLOAD(glLinkProgram);
    GLLOAD(glGetProgramiv); GLLOAD(glUseProgram); GLLOAD(glGetUniformLocation);
    GLLOAD(glUniform4f); GLLOAD(glDrawArrays); GLLOAD(glDeleteTextures);
    GLLOAD(glDeleteFramebuffers); GLLOAD(glDeleteBuffers); GLLOAD(glDeleteProgram);
    GLLOAD(glDeleteShader);
#undef GLLOAD
    return 0;
}
int main(void) {
    puts("=== MPFree GPU stage 2: offscreen 800x480 composite scene ===");
    puts("Never opens /dev/dri/card*, never sets a display mode.");
    if (setpriority(PRIO_PROCESS,0,10)) perror("Nonfatal: setpriority");
    int rc = load_all();
    if (rc) return rc;
    EGLDisplay d = eglGetPlatformDisplayEXT(EGL_PLATFORM_SURFACELESS_MESA,NULL,NULL);
    EGLint major=0,minor=0;
    if (!d || !eglInitialize(d,&major,&minor)) {
        fprintf(stderr,"Surfaceless EGL initialization failed: 0x%x\n",(unsigned)eglGetError()); return 2;
    }
    printf("EGL: %d.%d %s\n",major,minor,eglQueryString(d,EGL_VENDOR));
    const char *ext = eglQueryString(d,EGL_EXTENSIONS);
    if (!ext || !strstr(ext,"EGL_KHR_surfaceless_context")) {
        fputs("EGL_KHR_surfaceless_context unavailable\n",stderr); return 3;
    }
    const EGLint cfgattr[] = { EGL_SURFACE_TYPE,EGL_PBUFFER_BIT,
        EGL_RENDERABLE_TYPE,EGL_OPENGL_ES2_BIT,
        EGL_RED_SIZE,8,EGL_GREEN_SIZE,8,EGL_BLUE_SIZE,8,EGL_ALPHA_SIZE,8,EGL_NONE };
    EGLConfig cfg=NULL; EGLint ncfg=0;
    if (!eglChooseConfig(d,cfgattr,&cfg,1,&ncfg) || !ncfg) {
        fputs("RGBA EGL config unavailable\n",stderr); return 3;
    }
    if (!eglBindAPI(EGL_OPENGL_ES_API)) { fputs("eglBindAPI failed\n",stderr); return 3; }
    const EGLint ctxattr[]={EGL_CONTEXT_CLIENT_VERSION,2,EGL_NONE};
    EGLContext c = eglCreateContext(d,cfg,NULL,ctxattr);
    if (!c || !eglMakeCurrent(d,NULL,NULL,c)) {
        fprintf(stderr,"Surfaceless eglMakeCurrent failed: 0x%x\n",(unsigned)eglGetError()); return 3;
    }
    const char *renderer=(const char *)glGetString(GL_RENDERER);
    printf("GL_RENDERER: %s\nGL_VERSION: %s\n",renderer?renderer:"(null)",
           (const char *)glGetString(GL_VERSION));
    if (!renderer || !strstr(renderer,"Panfrost")) {
        fputs("Refusing benchmark: GPU renderer is not Panfrost\n",stderr); return 4;
    }
    GLuint tex=0,fbo=0,vbo=0;
    glGenTextures(1,&tex); glBindTexture(GL_TEXTURE_2D,tex);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glTexImage2D(GL_TEXTURE_2D,0,GL_RGBA,WIDTH,HEIGHT,0,GL_RGBA,GL_UNSIGNED_BYTE,NULL);
    glGenFramebuffers(1,&fbo); glBindFramebuffer(GL_FRAMEBUFFER,fbo);
    glFramebufferTexture2D(GL_FRAMEBUFFER,GL_COLOR_ATTACHMENT0,GL_TEXTURE_2D,tex,0);
    GLenum fbstatus = glCheckFramebufferStatus(GL_FRAMEBUFFER);
    if (fbstatus!=GL_FRAMEBUFFER_COMPLETE) {
        fprintf(stderr,"Offscreen FBO incomplete: 0x%x\n",fbstatus); return 5;
    }
    const char *vert = "attribute vec2 a_position; void main(){ gl_Position=vec4(a_position,0.0,1.0); }";
    const char *frag = "precision mediump float; uniform vec4 u_color; void main(){ gl_FragColor=u_color; }";
    GLuint vs=glCreateShader(GL_VERTEX_SHADER), fs=glCreateShader(GL_FRAGMENT_SHADER);
    glShaderSource(vs,1,&vert,NULL); glShaderSource(fs,1,&frag,NULL);
    glCompileShader(vs); glCompileShader(fs);
    GLint okv=0,okf=0; glGetShaderiv(vs,GL_COMPILE_STATUS,&okv); glGetShaderiv(fs,GL_COMPILE_STATUS,&okf);
    if (!okv || !okf) { fprintf(stderr,"Shader compilation failed (VS=%d FS=%d)\n",okv,okf); return 6; }
    GLuint prog=glCreateProgram(); glAttachShader(prog,vs); glAttachShader(prog,fs);
    glBindAttribLocation(prog,0,"a_position"); glLinkProgram(prog);
    GLint okp=0; glGetProgramiv(prog,GL_LINK_STATUS,&okp);
    if (!okp) { fputs("Shader linking failed\n",stderr); return 6; }
    glUseProgram(prog); ucolor=glGetUniformLocation(prog,"u_color");
    if (ucolor<0) { fputs("u_color uniform missing\n",stderr); return 6; }
    glGenBuffers(1,&vbo); glBindBuffer(GL_ARRAY_BUFFER,vbo);
    glEnableVertexAttribArray(0); glVertexAttribPointer(0,2,GL_FLOAT,GL_FALSE,0,NULL);
    glViewport(0,0,WIDTH,HEIGHT);
    double samples[SAMPLES];
    double t0=mono_ms();
    for (int frame=0; frame<WARMUP+SAMPLES; ++frame) {
        double begin=mono_ms();
        scene(frame);
        glFinish();   /* Forces completion. Measures CPU submission + GPU wait, not scanout FPS. */
        double ms=mono_ms()-begin;
        if (frame>=WARMUP) samples[frame-WARMUP]=ms;
        if (glGetError()!=GL_NO_ERROR) { fputs("GL error in scene rendering\n",stderr); return 7; }
        /* Pace the benchmark; limit interference with the realtime audio workload. */
        struct timespec sleep_for={0,12000000L};
        nanosleep(&sleep_for,NULL);
    }
    double wall_ms=mono_ms()-t0;
    qsort(samples,SAMPLES,sizeof(samples[0]),cmp_double);
    double sum=0; for(int i=0;i<SAMPLES;++i) sum+=samples[i];
    printf("Measured frames: %d (plus %d warmup)\n",SAMPLES,WARMUP);
    printf("Resolution: %dx%d RGBA8 FBO\n",WIDTH,HEIGHT);
    printf("Workload: 4x256 waveforms, 64 spectrum bars, 16x8 sequencer cells; dynamic VBO updates\n");
    printf("Synchronized frame time ms: avg=%.3f p50=%.3f p95=%.3f p99=%.3f max=%.3f\n",
           sum/SAMPLES,samples[SAMPLES/2],samples[(SAMPLES*95)/100],samples[(SAMPLES*99)/100],samples[SAMPLES-1]);
    printf("Wall time including 12ms frame pacing: %.1f ms\n",wall_ms);
    printf("NOTE: Includes glFinish() stalls; not display FPS or hardware GPU-only timing.\n");
    rc=snapshot("/tmp/mpfree-gpu-bench.ppm");
    printf("Preview snapshot: %s\n",rc==0?"/tmp/mpfree-gpu-bench.ppm":"FAILED");
    FILE *status=fopen("/proc/self/status","r");
    if (status) {
        char line[256];
        while (fgets(line,sizeof(line),status)) {
            if (!strncmp(line,"VmRSS:",6) || !strncmp(line,"VmHWM:",6)) printf("%s",line);
        }
        fclose(status);
    }
    glBindFramebuffer(GL_FRAMEBUFFER,0);
    glDeleteBuffers(1,&vbo); glDeleteFramebuffers(1,&fbo); glDeleteTextures(1,&tex);
    glDeleteProgram(prog); glDeleteShader(vs); glDeleteShader(fs);
    eglMakeCurrent(d,NULL,NULL,NULL); eglDestroyContext(d,c); eglTerminate(d);
    if (rc!=0) return rc;
    puts("RESULT: PASS — real GPU composite scene rendered to offscreen framebuffer.");
    return 0;
}