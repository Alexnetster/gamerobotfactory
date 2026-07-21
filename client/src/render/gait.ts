export interface LegAngles {
  hipDeg: number
  kneeDeg: number
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t
}

const STANCE_END = 0.6
const KNEE_PEAK = 0.8
const HIP_STANCE_START_DEG = 9
const HIP_STANCE_END_DEG = -8
const KNEE_PEAK_DEG = 28

/** 위상(0~1, 걸음 한 주기)을 (엉덩이, 무릎) 각도(도)로 매핑한다.
 *
 * 0~60%(디딤): 발이 바닥에 붙은 채 엉덩이가 천천히 뒤로 기울며 몸통을
 * 앞으로 옮긴다 — 무릎은 편 상태(0도) 유지.
 * 60~100%(흔듦): 엉덩이가 빠르게 앞으로 복귀하고, 무릎이 굽혀져(최대
 * `KNEE_PEAK_DEG`) 발을 들어올렸다가 다시 편다.
 *
 * 대칭 사인파 하나로 다리를 흔들면 "미끄러지듯 이동한다"는 인상을 준다
 * (2026-07-21 렌더링 브레인스토밍에서 목업으로 재현하고 고친 문제) —
 * 디딤/흔듦의 비대칭 타이밍과 무릎이 흔듦 구간에서만 굽는다는 점이 그
 * 문제를 해결하는 핵심이다.
 */
export function legAnglesForPhase(phase: number): LegAngles {
  const p = ((phase % 1) + 1) % 1

  const hipDeg = p <= STANCE_END
    ? lerp(HIP_STANCE_START_DEG, HIP_STANCE_END_DEG, p / STANCE_END)
    : lerp(HIP_STANCE_END_DEG, HIP_STANCE_START_DEG, (p - STANCE_END) / (1 - STANCE_END))

  let kneeDeg: number
  if (p <= STANCE_END) {
    kneeDeg = 0
  } else if (p <= KNEE_PEAK) {
    kneeDeg = lerp(0, KNEE_PEAK_DEG, (p - STANCE_END) / (KNEE_PEAK - STANCE_END))
  } else {
    kneeDeg = lerp(KNEE_PEAK_DEG, 0, (p - KNEE_PEAK) / (1 - KNEE_PEAK))
  }

  return { hipDeg, kneeDeg }
}
