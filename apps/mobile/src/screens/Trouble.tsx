import type { ExchangeView } from '@yuppers/api-client';
import {
  labelText,
  moveWording,
  TROUBLE_SITUATIONS,
  troubleRoute,
  troubleSituationOf,
  type Actions as ExchangeActions,
  type TroubleOffer,
  type TroubleSituation,
  type TroubleWay,
  type Wording,
} from '@yuppers/shared';
import { useEffect, useRef, useState } from 'react';
import { StyleSheet, Text, View } from 'react-native';

import { HelpLink } from '../components/HelpLink';
import { Actions, Button, Heading, Notice, P, Panel, Written } from '../components/ui';
import { focusOn } from '../lib/accessibility';
import { useI18n } from '../lib/context';
import { space, type, useColors } from '../lib/theme';

interface Props {
  exchange: ExchangeView;
  otherName: string;
  actions: ExchangeActions;
  /** Opens the composer, for a change to the agreement. */
  onRevise(): void;
}

/** What each way forward is called on its button: the action's own name. */
function wayLabel(wording: Wording, way: TroubleWay, money: boolean): string {
  switch (way) {
    case 'DISPUTE':
    case 'WAIVE':
    case 'RECLAIM':
      return moveWording(wording, way, money);
    case 'PROPOSE_END':
      return wording.exchange.proposeEnd;
    case 'AGREE_END':
      return wording.exchange.agreeEnd;
    case 'REQUEST_CLOSE':
      return wording.exchange.requestClose;
    case 'AMEND':
      return wording.exchange.amend;
  }
}

/**
 * "Something isn't working" (DESIGN.md §5.3): asks what the situation is,
 * says what each way forward means for who is released from what, and opens
 * the existing action the person picks, where they confirm it as usual. It
 * sends nothing itself.
 */
export function Trouble({ exchange, otherName, actions, onRevise }: Props) {
  const { wording, fmt, language } = useI18n();
  const colors = useColors();
  const t = wording.trouble;
  const [situation, setSituation] = useState<TroubleSituation | null>(
    troubleSituationOf(actions.panel) ?? null,
  );
  // A situation chosen here takes the screen reader to what it says.
  const [chosen, setChosen] = useState(false);
  const explained = useRef<Text>(null);
  useEffect(() => {
    if (chosen && situation) focusOn(explained.current as unknown as View | null);
  }, [chosen, situation]);

  const route = situation ? troubleRoute(exchange, situation) : null;

  return (
    <Panel title={t.open}>
      {!situation || !route ? (
        <>
          <P>{t.intro}</P>
          <Heading level={4}>{t.question}</Heading>
          <View style={styles.choices}>
            {TROUBLE_SITUATIONS.map((option) => (
              <Button
                key={option}
                label={fmt(t.situations[option], { name: otherName })}
                accessibilityLabel={fmt(t.situations[option], { name: labelText(otherName) })}
                onPress={() => {
                  setChosen(true);
                  setSituation(option);
                }}
              />
            ))}
          </View>
        </>
      ) : (
        <>
          <Heading level={4}>{fmt(t.situations[situation], { name: otherName })}</Heading>
          <Text
            ref={explained}
            accessibilityLanguage={language}
            style={[type.body, { color: colors.text }]}>
            {fmt(t.explain[situation], { name: otherName })}
          </Text>
          {route.notes.map((note) => (
            <Notice key={note} quiet>
              {fmt(t[note], { name: otherName })}
            </Notice>
          ))}
          {route.offers.map((offer) => (
            <Way
              key={offer.way}
              offer={offer}
              otherName={otherName}
              actions={actions}
              onRevise={onRevise}
            />
          ))}
        </>
      )}
      <HelpLink place="trouble" />
      <Actions>
        {situation ? (
          <Button
            variant="link"
            label={t.change}
            onPress={() => {
              setChosen(false);
              setSituation(null);
            }}
          />
        ) : null}
        <Button label={wording.common.cancel} onPress={actions.close} />
      </Actions>
    </Panel>
  );
}

interface WayProps {
  offer: TroubleOffer;
  otherName: string;
  actions: ExchangeActions;
  onRevise(): void;
}

/** One way forward: what it means, then the action, on each item it can be taken on. */
function Way({ offer, otherName, actions, onRevise }: WayProps) {
  const { wording, fmt } = useI18n();
  const colors = useColors();
  const means = fmt(wording.trouble.means[offer.way], { name: otherName });

  return (
    <View style={[styles.way, { borderTopColor: colors.divider }]}>
      <P>{means}</P>
      {offer.items.map((item) => (
        <View key={item.id} style={styles.item}>
          <Written>{item.description}</Written>
          <Actions>
            <Button
              label={wayLabel(wording, offer.way, item.money)}
              // Which item it acts on, since the same action may be offered on several.
              hint={labelText(item.description)}
              disabled={actions.busy}
              onPress={() => actions.open(item.panel)}
            />
          </Actions>
        </View>
      ))}
      {offer.items.length === 0 && (
        <Actions>
          <Button
            label={wayLabel(wording, offer.way, false)}
            disabled={actions.busy}
            onPress={() => (offer.panel ? actions.open(offer.panel) : onRevise())}
          />
        </Actions>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  choices: { gap: space.s },
  way: { borderTopWidth: StyleSheet.hairlineWidth, paddingTop: space.m, gap: space.s },
  item: { gap: space.xs },
});
