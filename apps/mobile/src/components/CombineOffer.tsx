import type { ErrorCode } from '@yuppers/api-client';
import {
  combineAccountLines,
  combineEffects,
  combineHeading,
  type CombineOffer as Offer,
} from '@yuppers/shared';

import { useI18n } from '../lib/context';
import { Actions, Button, Failure, Heading, P, Panel } from './ui';

interface Props {
  offer: Offer;
  busy: boolean;
  failure: ErrorCode | null;
  onCombine(): void;
  onCancel(): void;
}

/**
 * The offer to combine another account into this one, as on the web
 * (`apps/web/src/components/CombineOffer.tsx`): what that account is, what
 * moves and what is dropped, that it cannot be undone, and one deliberate
 * button.
 */
export function CombineOffer({ offer, busy, failure, onCombine, onCancel }: Props) {
  const { wording, language } = useI18n();
  const w = wording.combine;
  return (
    <Panel title={combineHeading(w, offer)}>
      <P>{w.intro}</P>
      <Heading level={4}>{w.otherHeading}</Heading>
      {combineAccountLines(w, offer, language).map((line) => (
        <P key={line}>{`• ${line}`}</P>
      ))}
      <Heading level={4}>{w.movesHeading}</Heading>
      {combineEffects(w, offer, language).map((line) => (
        <P key={line}>{`• ${line}`}</P>
      ))}
      <P>{`${w.cannotUndo} ${w.expires}`}</P>
      <Failure code={failure} />
      <Actions>
        <Button label={w.confirm} variant="primary" disabled={busy} onPress={onCombine} />
        <Button label={wording.common.cancel} disabled={busy} onPress={onCancel} />
      </Actions>
    </Panel>
  );
}
