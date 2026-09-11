import debug from 'debug';
import { useEffect, useLayoutEffect, useMemo, useRef } from 'react';

import { useAppSelector } from '../../../store/hooks';
import {
  selectChatMascotExpanded,
  selectChatMascotListening,
  selectCustomMascotGifUrl,
  selectCustomPrimaryColor,
  selectCustomSecondaryColor,
  selectMascotColor,
  selectSpeakReplies,
} from '../../../store/mascotSlice';
import {
  CustomGifMascot,
  getMascotPalette,
  hexToArgbInt,
  ManifestRiveMascot,
  RiveMascot,
} from '../Mascot';
import { useMascotManifest } from '../Mascot/manifest/useMascotManifest';
import { useHumanMascot } from '../useHumanMascot';
import { useChatMascot, useDockNode } from './ChatMascotContext';
import {
  boxTransform,
  easeInOutCubic,
  inscribedSquare,
  lerpBox,
  type MascotBox,
  prefersReducedMotion,
  snapBox,
  STAGE_RENDER_PX,
  TRANSITION_MS,
} from './geometry';

const overlayLog = debug('human:chat-mascot');

/** Off-screen box used until both anchors have been measured. */
const HIDDEN_BOX: MascotBox = { left: -9999, top: -9999, size: 0 };

/** How often to re-check for an anchor that has not mounted yet, and for how long. */
const ANCHOR_POLL_MS = 100;
const ANCHOR_POLL_TIMEOUT_MS = 3_000;

function measure(el: HTMLElement | null): MascotBox | null {
  if (!el) return null;
  const rect = el.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) return null;
  return inscribedSquare(rect);
}

/**
 * The one and only chat mascot.
 *
 * Rendered in a fixed-position box that is *always* laid out as a constant
 * `STAGE_RENDER_PX` square at the viewport origin, then moved onto either the
 * composer dock or the voice stage with a CSS `transform`. Two consequences,
 * both deliberate:
 *
 *  - **One Rive instance.** Mounting a second mascot for the expanded state
 *    would load the `.riv` twice and turn the transition into a crossfade
 *    rather than the mascot actually travelling and scaling.
 *  - **The canvas never resizes.** Animating width/height would rebuild the
 *    canvas backing store every frame; a transform is composited. It is laid out
 *    once at `STAGE_RENDER_PX` — roughly the size the old Human page gave the
 *    mascot — and only ever scaled DOWN onto an anchor. Scaling a canvas up is a
 *    raster stretch, which is what made the staged mascot look soft.
 *
 * The travel itself is driven by a `requestAnimationFrame` loop rather than a
 * CSS transition, because the stage column's own width animation moves the
 * target while the mascot is in flight — re-measuring per frame keeps the
 * mascot glued to its destination instead of drifting and snapping at the end.
 *
 * This component re-renders at ~60fps while the agent speaks (lipsync). It is a
 * leaf on purpose: nothing else may be rendered underneath it. See #5357.
 *
 * [ui-flow] chat-mascot-overlay: docked ⇄ staged (rAF travel, TRANSITION_MS)
 */
const ChatMascotOverlay = () => {
  const { dockRef, stageRef } = useChatMascot();
  const dockNode = useDockNode();
  const expanded = useAppSelector(selectChatMascotExpanded);
  const listening = useAppSelector(selectChatMascotListening);
  const speakRepliesPref = useAppSelector(selectSpeakReplies);

  // Speech is a property of the *stage*, not of chat in general: a docked
  // mascot must not start talking over a text conversation just because the
  // preference defaults on. Collapsing therefore silences replies, and the
  // switch in the stage governs the expanded state.
  const speakReplies = speakRepliesPref && expanded;

  const { face, visemeCode } = useHumanMascot({ speakReplies, listening });

  const mascotColor = useAppSelector(selectMascotColor);
  const customPrimary = useAppSelector(selectCustomPrimaryColor);
  const customSecondary = useAppSelector(selectCustomSecondaryColor);
  const customMascotGifUrl = useAppSelector(selectCustomMascotGifUrl);
  const { entry: mascotEntry } = useMascotManifest();

  const palette = getMascotPalette(mascotColor);
  const primaryColor = useMemo(
    () => hexToArgbInt(mascotColor === 'custom' ? customPrimary : palette.bodyFill),
    [mascotColor, customPrimary, palette]
  );
  const secondaryColor = useMemo(
    () => hexToArgbInt(mascotColor === 'custom' ? customSecondary : palette.neckShadowColor),
    [mascotColor, customSecondary, palette]
  );

  const boxRef = useRef<HTMLDivElement | null>(null);
  // Where the mascot currently sits, so an interrupted transition resumes from
  // the on-screen position instead of jumping back to the anchor it left.
  const currentBoxRef = useRef<MascotBox>(HIDDEN_BOX);
  const rafRef = useRef<number | null>(null);

  // Apply a box imperatively. Deliberately not React state: this runs every
  // frame of the travel, and a setState per frame would re-render the mascot
  // (and reconcile Rive's props) 60 times a second for no visual gain.
  //
  // `travelling` controls `will-change`, and getting that wrong is what makes
  // the mascot look like a blown-up thumbnail. `will-change: transform` promotes
  // the element to a composited layer that the compositor rasterises ONCE, at
  // whatever scale it first sees. The mascot mounts docked at ~0.15, so a
  // permanently-hinted layer bakes a ~64px texture and then stretches it to fill
  // the 420px stage. Hinting only for the duration of the travel lets the
  // compositor re-raster at the resting scale, which is where it is looked at.
  const applyBox = (box: MascotBox, travelling = false) => {
    // Sub-pixel precision keeps the travel smooth; at rest it just resamples the
    // canvas across the pixel grid, so the landing position is snapped.
    const applied = travelling ? box : snapBox(box);
    currentBoxRef.current = applied;
    const el = boxRef.current;
    if (!el) return;

    if (travelling) {
      // Mid-flight: a fixed layout box moved by transform. Cheap (composited,
      // no canvas reallocation) and any softness is invisible while moving.
      el.style.width = `${STAGE_RENDER_PX}px`;
      el.style.height = `${STAGE_RENDER_PX}px`;
      el.style.transform = boxTransform(applied, STAGE_RENDER_PX);
      el.style.willChange = 'transform';
    } else {
      // At rest: lay the mascot out at its TRUE size and translate only — no
      // scale at all.
      //
      // This is not a nicety. Rive's renderer sizes its canvas backing store
      // from `getBoundingClientRect()`, which includes ancestor transforms, and
      // `ResizeObserver` does not fire on transform changes. So a permanently
      // transform-scaled canvas gets a backing store matching whatever scale it
      // happened to be measured at — docked, that is ~64px — and expanding only
      // changes the transform, so Rive never re-measures and stretches that
      // 64px texture across the whole stage. Measured: backing 127x127 for a
      // 768px box. Changing the layout size instead makes the observer fire and
      // Rive reallocate at the right resolution.
      el.style.width = `${applied.size}px`;
      el.style.height = `${applied.size}px`;
      el.style.transform = `translate3d(${applied.left}px, ${applied.top}px, 0)`;
      el.style.willChange = 'auto';
    }
    el.style.opacity = applied.size > 0 ? '1' : '0';
  };

  useLayoutEffect(() => {
    const readTarget = (): MascotBox | null =>
      measure(expanded ? stageRef.current : dockRef.current);

    const settle = () => {
      const target = readTarget();
      if (target) applyBox(target);
    };

    const from = currentBoxRef.current;
    const initialTarget = readTarget();

    // The anchor is not measurable yet — it mounts a tick later, or the column
    // it lives in is still laying out. Park off-screen and poll until it
    // appears. A ResizeObserver cannot cover this: it only watches elements that
    // already exist, so an anchor that shows up afterwards would never wake
    // anything and the mascot would stay invisible for good.
    //
    // Polled on a coarse interval rather than per frame: nothing is moving yet,
    // so frame accuracy buys nothing, and it is bounded so a surface that never
    // mounts an anchor cannot leave a timer spinning for the session.
    if (!initialTarget) {
      applyBox(HIDDEN_BOX);
      let waited = 0;
      const poll = window.setInterval(() => {
        waited += ANCHOR_POLL_MS;
        const target = readTarget();
        if (target) {
          overlayLog('[chat-mascot][overlay] anchor appeared expanded=%s', expanded);
          applyBox(target);
          window.clearInterval(poll);
          return;
        }
        if (waited >= ANCHOR_POLL_TIMEOUT_MS) {
          overlayLog('[chat-mascot][overlay] gave up waiting for anchor expanded=%s', expanded);
          window.clearInterval(poll);
        }
      }, ANCHOR_POLL_MS);
      return () => window.clearInterval(poll);
    }

    // First placement, or reduced motion: no travel, just land.
    if (from.size <= 0 || prefersReducedMotion()) {
      overlayLog('[chat-mascot][overlay] snap expanded=%s size=%d', expanded, initialTarget.size);
      applyBox(initialTarget);
      return;
    }

    overlayLog('[chat-mascot][overlay] travel start expanded=%s', expanded);
    const startedAt = window.performance.now();
    const tick = (now: number) => {
      const t = easeInOutCubic((now - startedAt) / TRANSITION_MS);
      // Re-measure every frame: the stage column animates its own width, so the
      // destination is still moving while we fly toward it.
      const target = readTarget() ?? initialTarget;
      applyBox(lerpBox(from, target, t), true);
      if (t < 1) {
        rafRef.current = window.requestAnimationFrame(tick);
        return;
      }
      rafRef.current = null;
      overlayLog('[chat-mascot][overlay] travel done expanded=%s', expanded);
      settle();
    };
    rafRef.current = window.requestAnimationFrame(tick);

    return () => {
      if (rafRef.current != null) {
        window.cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  }, [expanded, dockRef, stageRef]);

  // Keep the mascot on its anchor when the layout moves underneath it (window
  // resize, sidebar collapse, composer growing with a multiline draft). Skipped
  // while a travel is in flight — that loop is already re-measuring.
  useEffect(() => {
    const resettle = () => {
      if (rafRef.current != null) return;
      const target = measure(expanded ? stageRef.current : dockRef.current);
      if (target) applyBox(target);
    };

    window.addEventListener('resize', resettle);
    const observer = new ResizeObserver(resettle);
    const stage = stageRef.current;
    if (stage) observer.observe(stage);
    if (document.body) observer.observe(document.body);

    // The dock's own size never changes, so observing it alone would miss every
    // way it *moves*: a multiline draft growing the textarea, an attachment
    // strip appearing, the queued-follow-ups panel or the goal editor opening.
    // Each of those resizes an ancestor of the dock, so walk a few levels up —
    // that covers the composer's input box and its surrounding footer without
    // resorting to a permanent per-frame position poll.
    //
    // Driven by `dockNode` (state), not `dockRef.current`: this effect must
    // re-subscribe when the dock mounts late or remounts, otherwise it observes
    // nothing — or keeps watching detached nodes — for the rest of the session.
    const DOCK_ANCESTORS_WATCHED = 3;
    let node: HTMLElement | null = dockNode;
    for (let i = 0; node && i <= DOCK_ANCESTORS_WATCHED; i += 1) {
      observer.observe(node);
      node = node.parentElement;
    }

    return () => {
      window.removeEventListener('resize', resettle);
      observer.disconnect();
    };
  }, [expanded, dockNode, dockRef, stageRef]);

  return (
    <div
      ref={boxRef}
      aria-hidden="true"
      // The real controls are the dock button and the stage's collapse button;
      // this layer is decoration and must never swallow a click on either.
      className="pointer-events-none fixed left-0 top-0 z-30"
      style={{
        width: STAGE_RENDER_PX,
        height: STAGE_RENDER_PX,
        transformOrigin: 'top left',
        opacity: 0,
      }}
      data-testid="chat-mascot-overlay"
      data-expanded={expanded ? 'true' : 'false'}>
      {customMascotGifUrl ? (
        <CustomGifMascot src={customMascotGifUrl} face={face} />
      ) : mascotEntry ? (
        <ManifestRiveMascot
          key={mascotEntry.id}
          entry={mascotEntry}
          face={face}
          primaryColor={primaryColor}
          secondaryColor={secondaryColor}
          visemeCode={visemeCode}
          idlePoseRotation
        />
      ) : (
        <RiveMascot
          face={face}
          primaryColor={primaryColor}
          secondaryColor={secondaryColor}
          visemeCode={visemeCode}
          idlePoseRotation
        />
      )}
    </div>
  );
};

export default ChatMascotOverlay;
