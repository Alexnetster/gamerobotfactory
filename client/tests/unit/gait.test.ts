import { describe, it, expect } from 'vitest'
import { legAnglesForPhase } from '../../src/render/gait'

describe('legAnglesForPhase', () => {
  it('디딤 시작(위상 0)에서 엉덩이는 앞으로 기울고 무릎은 편 상태', () => {
    const { hipDeg, kneeDeg } = legAnglesForPhase(0)
    expect(hipDeg).toBeCloseTo(9, 5)
    expect(kneeDeg).toBeCloseTo(0, 5)
  })

  it('디딤 끝(위상 0.6)에서 엉덩이는 뒤로 기울고 무릎은 여전히 편 상태', () => {
    const { hipDeg, kneeDeg } = legAnglesForPhase(0.6)
    expect(hipDeg).toBeCloseTo(-8, 5)
    expect(kneeDeg).toBeCloseTo(0, 5)
  })

  it('디딤 구간 중간(위상 0.3)에서 무릎은 계속 0도(편 상태)', () => {
    const { hipDeg, kneeDeg } = legAnglesForPhase(0.3)
    expect(hipDeg).toBeCloseTo(0.5, 5)
    expect(kneeDeg).toBe(0)
  })

  it('흔듦 구간 정점(위상 0.8)에서 무릎이 최대로 굽음(발을 들어올림)', () => {
    const { hipDeg, kneeDeg } = legAnglesForPhase(0.8)
    expect(hipDeg).toBeCloseTo(0.5, 5)
    expect(kneeDeg).toBeCloseTo(28, 5)
  })

  it('흔듦 구간 후반(위상 0.9)에서 엉덩이는 앞으로 복귀 중, 무릎은 절반쯤 펴짐', () => {
    const { hipDeg, kneeDeg } = legAnglesForPhase(0.9)
    expect(hipDeg).toBeCloseTo(4.75, 5)
    expect(kneeDeg).toBeCloseTo(14, 5)
  })

  it('위상 1.0은 위상 0과 같다(한 주기가 매끈하게 이어짐)', () => {
    const atOne = legAnglesForPhase(1.0)
    const atZero = legAnglesForPhase(0)
    expect(atOne.hipDeg).toBeCloseTo(atZero.hipDeg, 5)
    expect(atOne.kneeDeg).toBeCloseTo(atZero.kneeDeg, 5)
  })

  it('음수 위상도 정규화되어 0.9와 같은 결과를 낸다', () => {
    const negative = legAnglesForPhase(-0.1)
    const equivalent = legAnglesForPhase(0.9)
    expect(negative.hipDeg).toBeCloseTo(equivalent.hipDeg, 5)
    expect(negative.kneeDeg).toBeCloseTo(equivalent.kneeDeg, 5)
  })

  it('디딤 구간(위상 0~0.6) 전체에서 무릎은 항상 0도 — 흔듦 구간에서만 굽는다는 비대칭 타이밍의 핵심 불변식', () => {
    for (const p of [0, 0.1, 0.2, 0.4, 0.59]) {
      expect(legAnglesForPhase(p).kneeDeg).toBe(0)
    }
  })
})
