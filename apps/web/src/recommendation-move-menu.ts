export const RECOMMENDATION_MOVE_MENU_COLLISION_PADDING = 12
export const RECOMMENDATION_MOVE_MENU_MAX_WIDTH = 288
const MENU_GAP = 6
const ESTIMATED_MENU_HEIGHT = 112

export interface RecommendationMoveMenuPosition {
  left: number
  maxHeight: number
  placement: 'above' | 'below'
  top: number
  width: number
}

export interface RecommendationMoveMenuViewport {
  height: number
  offsetLeft: number
  offsetTop: number
  width: number
}

export type RecommendationMoveMenuPlacement =
  | { position: RecommendationMoveMenuPosition; status: 'visible' }
  | { status: 'reference-hidden' }

export function positionRecommendationMoveMenu(
  trigger: Pick<DOMRect, 'bottom' | 'height' | 'left' | 'right' | 'top' | 'width'>,
  viewport: RecommendationMoveMenuViewport,
): RecommendationMoveMenuPlacement {
  const padding = RECOMMENDATION_MOVE_MENU_COLLISION_PADDING
  const viewportLeft = viewport.offsetLeft
  const viewportTop = viewport.offsetTop
  const viewportRight = viewportLeft + viewport.width
  const viewportBottom = viewportTop + viewport.height
  const measurable = trigger.width > 0 || trigger.height > 0
  if (
    measurable
    && (
    trigger.bottom <= viewportTop
    || trigger.top >= viewportBottom
    || trigger.right <= viewportLeft
    || trigger.left >= viewportRight
    )
  ) {
    return { status: 'reference-hidden' }
  }
  const width = Math.min(
    RECOMMENDATION_MOVE_MENU_MAX_WIDTH,
    Math.max(0, viewport.width - padding * 2),
  )
  const left = Math.min(
    Math.max(trigger.left, viewportLeft + padding),
    Math.max(viewportLeft + padding, viewportRight - padding - width),
  )
  const availableBelow = Math.max(0, viewportBottom - trigger.bottom - MENU_GAP - padding)
  const availableAbove = Math.max(0, trigger.top - MENU_GAP - viewportTop - padding)
  const placement = availableBelow >= Math.min(ESTIMATED_MENU_HEIGHT, availableAbove)
    ? 'below'
    : 'above'
  const maxHeight = placement === 'below' ? availableBelow : availableAbove
  const renderedHeight = Math.min(ESTIMATED_MENU_HEIGHT, maxHeight)
  const proposedTop = placement === 'below'
    ? trigger.bottom + MENU_GAP
    : trigger.top - MENU_GAP - renderedHeight
  const top = Math.min(
    Math.max(proposedTop, viewportTop + padding),
    Math.max(viewportTop + padding, viewportBottom - padding - renderedHeight),
  )

  return {
    position: { left, maxHeight, placement, top, width },
    status: 'visible',
  }
}
