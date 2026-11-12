/* Compile-time check that mpfree-app's ALSA constants (src/alsa.rs) match the
 * kernel's <sound/asound.h> for the MPC One's 32-bit ARM. CI compiles it with
 * arm-linux-gnueabihf-gcc -c; a mismatch stops the build. Nothing runs. */
#include <stddef.h>
#include <sys/ioctl.h>
#include <sound/asound.h>
#include <linux/input.h>

#define SAME(a, b) _Static_assert((a) == (b), #a " != " #b)

SAME(sizeof(struct snd_pcm_hw_params), 604);
SAME(sizeof(struct snd_pcm_sw_params), 104);
SAME(sizeof(struct snd_xferi), 12);
SAME(sizeof(struct snd_rawmidi_info), 268);
SAME(sizeof(struct input_event), 16);

SAME(offsetof(struct snd_pcm_hw_params, masks), 4);
SAME(offsetof(struct snd_pcm_hw_params, intervals), 260);
SAME(offsetof(struct snd_pcm_hw_params, rmask), 512);
SAME(offsetof(struct snd_pcm_hw_params, cmask), 516);
SAME(offsetof(struct snd_pcm_hw_params, info), 520);
SAME(offsetof(struct snd_pcm_hw_params, msbits), 524);
SAME(offsetof(struct snd_pcm_hw_params, rate_num), 528);
SAME(offsetof(struct snd_pcm_hw_params, rate_den), 532);
SAME(offsetof(struct snd_pcm_hw_params, fifo_size), 536);

SAME(offsetof(struct snd_pcm_sw_params, avail_min), 12);
SAME(offsetof(struct snd_pcm_sw_params, start_threshold), 20);
SAME(offsetof(struct snd_pcm_sw_params, stop_threshold), 24);
SAME(offsetof(struct snd_pcm_sw_params, boundary), 36);

SAME(offsetof(struct snd_xferi, result), 0);
SAME(offsetof(struct snd_xferi, buf), 4);
SAME(offsetof(struct snd_xferi, frames), 8);

SAME(offsetof(struct snd_rawmidi_info, subdevice), 4);
SAME(offsetof(struct snd_rawmidi_info, stream), 8);
SAME(offsetof(struct snd_rawmidi_info, name), 84);
SAME(offsetof(struct snd_rawmidi_info, subname), 164);
SAME(offsetof(struct snd_rawmidi_info, subdevices_count), 196);

SAME(offsetof(struct input_event, type), 8);
SAME(offsetof(struct input_event, code), 10);
SAME(offsetof(struct input_event, value), 12);

SAME(SNDRV_PCM_IOCTL_HW_REFINE, 0xC25C4110u);
SAME(SNDRV_PCM_IOCTL_HW_PARAMS, 0xC25C4111u);
SAME(SNDRV_PCM_IOCTL_SW_PARAMS, 0xC0684113u);
SAME(SNDRV_PCM_IOCTL_PREPARE, 0x4140u);
SAME(SNDRV_PCM_IOCTL_DROP, 0x4143u);
SAME(SNDRV_PCM_IOCTL_DRAIN, 0x4144u);
SAME(SNDRV_PCM_IOCTL_WRITEI_FRAMES, 0x400C4150u);
SAME(SNDRV_PCM_IOCTL_DELAY, 0x80044121u);
SAME(SNDRV_CTL_IOCTL_RAWMIDI_PREFER_SUBDEVICE, 0x40045542u);
SAME(SNDRV_RAWMIDI_IOCTL_INFO, 0x810C5701u);

SAME(SNDRV_PCM_ACCESS_RW_INTERLEAVED, 3);
SAME(SNDRV_RAWMIDI_STREAM_OUTPUT, 0);
SAME(SNDRV_RAWMIDI_STREAM_INPUT, 1);
SAME(SNDRV_PCM_FORMAT_S32_LE, 10);
SAME(SNDRV_PCM_SUBFORMAT_STD, 0);
SAME(SNDRV_PCM_HW_PARAM_SAMPLE_BITS, 8);
SAME(SNDRV_PCM_HW_PARAM_FRAME_BITS, 9);
SAME(SNDRV_PCM_HW_PARAM_CHANNELS, 10);
SAME(SNDRV_PCM_HW_PARAM_RATE, 11);
SAME(SNDRV_PCM_HW_PARAM_PERIOD_SIZE, 13);
SAME(SNDRV_PCM_HW_PARAM_PERIODS, 15);
SAME(SNDRV_PCM_HW_PARAM_BUFFER_SIZE, 17);
SAME(SNDRV_PCM_HW_PARAM_LAST_INTERVAL, 19);

SAME(EV_SYN, 0);
SAME(EV_ABS, 3);
SAME(SYN_REPORT, 0);
SAME(ABS_MT_SLOT, 0x2f);
SAME(ABS_MT_POSITION_X, 0x35);
SAME(ABS_MT_POSITION_Y, 0x36);
SAME(ABS_MT_TRACKING_ID, 0x39);
