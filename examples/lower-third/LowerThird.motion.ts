import { spring, timeline, tween } from '@kine/core';
import { LowerThird } from './LowerThird';

export default timeline(LowerThird, ({ at }) => ({
  keys: [
    at(0.0, 'panel', { x: [-40, 0], opacity: [0, 1] }, spring.snappy),
    at(0.15, 'title', { reveal: [0, 1] }, tween(0.4, 'out')),
    at(0.35, 'subtitle', { opacity: [0, 1], y: [8, 0] }, tween(0.25, 'out')),
    at(4.0, 'panel', { opacity: 0, x: -20 }, tween(0.3, 'in')),
  ],
  markers: { loop: 1.2, out: 4.0 },
}));
