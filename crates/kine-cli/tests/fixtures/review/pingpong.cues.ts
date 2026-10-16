import { cues } from '@kine/core';
import scene, { Card } from './pingpong';
export default cues(scene, ({ at }) => [at(0, 'play'), at(1.5, 'invoke', Card, 'flash', {})]);
