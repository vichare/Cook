import { describe, expect, it } from 'vitest'
import {
  RECOMMENDATION_MOVE_MENU_COLLISION_PADDING,
  RECOMMENDATION_MOVE_MENU_MAX_WIDTH,
  positionRecommendationMoveMenu,
} from './recommendation-move-menu'

describe('recommendation move menu positioning', () => {
  it('clamps a first-chip menu inside a 320px viewport with collision padding', () => {
    const position = positionRecommendationMoveMenu(
      { bottom: 124, height: 44, left: -20, right: 24, top: 80, width: 44 },
      { height: 640, offsetLeft: 0, offsetTop: 0, width: 320 },
    )

    expect(position.status).toBe('visible')
    if (position.status !== 'visible') throw new Error('expected visible placement')
    expect(position.position.left).toBe(RECOMMENDATION_MOVE_MENU_COLLISION_PADDING)
    expect(position.position.width).toBe(RECOMMENDATION_MOVE_MENU_MAX_WIDTH)
    expect(position.position.left + position.position.width).toBeLessThanOrEqual(
      320 - RECOMMENDATION_MOVE_MENU_COLLISION_PADDING,
    )
    expect(position.position.placement).toBe('below')
  })

  it('places the menu above when the lower viewport cannot fit it', () => {
    const position = positionRecommendationMoveMenu(
      { bottom: 544, height: 44, left: 280, right: 324, top: 500, width: 44 },
      { height: 568, offsetLeft: 0, offsetTop: 0, width: 320 },
    )

    expect(position.status).toBe('visible')
    if (position.status !== 'visible') throw new Error('expected visible placement')
    expect(position.position.placement).toBe('above')
    expect(position.position.left).toBe(20)
    expect(position.position.top).toBeGreaterThanOrEqual(
      RECOMMENDATION_MOVE_MENU_COLLISION_PADDING,
    )
  })

  it('uses visual viewport offsets and clamps both vertical directions', () => {
    const position = positionRecommendationMoveMenu(
      { bottom: 252, height: 44, left: 118, right: 162, top: 208, width: 44 },
      { height: 240, offsetLeft: 100, offsetTop: 200, width: 280 },
    )

    expect(position.status).toBe('visible')
    if (position.status !== 'visible') throw new Error('expected visible placement')
    expect(position.position.left).toBeGreaterThanOrEqual(112)
    expect(position.position.left + position.position.width).toBeLessThanOrEqual(368)
    expect(position.position.top).toBeGreaterThanOrEqual(212)
    expect(position.position.top + Math.min(112, position.position.maxHeight))
      .toBeLessThanOrEqual(428)
  })

  it.each([
    { bottom: 190, height: 44, left: 120, right: 164, top: 146, width: 44 },
    { bottom: 500, height: 44, left: 120, right: 164, top: 456, width: 44 },
  ])('reports a reference outside the visual viewport as hidden', (trigger) => {
    expect(positionRecommendationMoveMenu(
      trigger,
      { height: 240, offsetLeft: 100, offsetTop: 200, width: 280 },
    )).toEqual({ status: 'reference-hidden' })
  })
})
