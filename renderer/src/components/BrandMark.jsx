import { convertFileSrc } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';

const BRAND_ANIMATION_URL = `${import.meta.env.BASE_URL}assets/brand-elephant.json`;
const IDLE_SEGMENT = [0, 135];
const CLICKED_SEGMENT = [150, 270];
const CLICKED_PREVIEW_MS = 4300;

export default function BrandMark() {
  const containerRef = useRef(null);
  const animationRef = useRef(null);
  const audioRef = useRef(null);
  const clickedRef = useRef(false);
  const lastVoiceIndexRef = useRef(-1);
  const voiceFilesRef = useRef(null);
  const [pinned, setPinned] = useState(false);

  useEffect(() => {
    if (!containerRef.current) {
      return undefined;
    }

    let cancelled = false;

    async function loadBrandAnimation() {
      const [{ default: lottie }, response] = await Promise.all([
        import('lottie-web/build/player/lottie_light'),
        fetch(BRAND_ANIMATION_URL)
      ]);
      if (!response.ok) {
        throw new Error(`加载品牌动画失败: ${response.status}`);
      }
      const animationData = await response.json();
      if (cancelled || !containerRef.current) {
        return;
      }
      const animation = lottie.loadAnimation({
        animationData,
        autoplay: false,
        container: containerRef.current,
        loop: true,
        renderer: 'svg',
        rendererSettings: {
          preserveAspectRatio: 'xMidYMid meet'
        }
      });
      animationRef.current = animation;

      const playIdle = () => {
        animation.loop = true;
        animation.playSegments(IDLE_SEGMENT, true);
      };

      const handleComplete = () => {
        if (!clickedRef.current) {
          return;
        }
        clickedRef.current = false;
        playIdle();
      };

      animation.addEventListener('DOMLoaded', playIdle);
      animation.addEventListener('complete', handleComplete);
      playIdle();
    }

    loadBrandAnimation().catch((err) => {
      console.error(err);
    });

    return () => {
      cancelled = true;
      audioRef.current?.pause();
      audioRef.current = null;
      animationRef.current?.destroy();
      animationRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (!pinned) {
      return undefined;
    }

    const timeoutId = window.setTimeout(() => {
      setPinned(false);
    }, CLICKED_PREVIEW_MS);
    return () => window.clearTimeout(timeoutId);
  }, [pinned]);

  async function loadVoiceFiles() {
    if (voiceFilesRef.current) {
      return voiceFilesRef.current;
    }

    try {
      const result = await window.api?.listBrandVoiceFiles?.();
      const files = Array.isArray(result?.files) ? result.files : [];
      voiceFilesRef.current = files;
      return files;
    } catch {
      voiceFilesRef.current = [];
      return [];
    }
  }

  function pickVoiceFile(files) {
    if (!files.length) {
      return null;
    }
    if (files.length === 1) {
      lastVoiceIndexRef.current = 0;
      return files[0];
    }

    let index = Math.floor(Math.random() * files.length);
    if (index === lastVoiceIndexRef.current) {
      index = (index + 1) % files.length;
    }
    lastVoiceIndexRef.current = index;
    return files[index];
  }

  async function playRandomVoice() {
    const files = await loadVoiceFiles();
    const filePath = pickVoiceFile(files);
    if (!filePath) {
      return;
    }

    try {
      const audio = audioRef.current || new Audio();
      audioRef.current = audio;
      audio.pause();
      audio.currentTime = 0;
      audio.src = convertFileSrc(filePath);
      audio.volume = 0.9;
      await audio.play();
    } catch {
      voiceFilesRef.current = null;
    }
  }

  function playClickedSegment() {
    setPinned(true);
    void playRandomVoice();
    const animation = animationRef.current;
    if (!animation) {
      return;
    }
    clickedRef.current = true;
    animation.loop = false;
    animation.playSegments(CLICKED_SEGMENT, true);
  }

  return (
    <button
      type="button"
      className={`brand-mark ${pinned ? 'is-pinned' : ''}`}
      aria-label="Codex Switch"
      onClick={playClickedSegment}
    >
      <span className="brand-mark-animation" ref={containerRef} />
    </button>
  );
}
