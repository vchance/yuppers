import type { components, ErrorCode } from '@yuppers/api-client'

import type { AmendmentEffect } from '../amendment'
import type { ChangedField, ItemChangeKind } from '../changes'
import type { ProblemCode } from '../draft'
import type { Move } from '../fulfillment'
import type { SummaryOutcome } from '../summary'
import type { HelpLinkPlace, HelpTopic } from '../help'
import type { TroubleSituation, TroubleWay } from '../trouble'

type Schemas = components['schemas']

/** Why a closed exchange closed, as the service names the reasons. */
export type ClosedReason =
  | 'WITHDRAWN'
  | 'DECLINED'
  | 'EXPIRED'
  | 'DISCARDED'
  | 'CLOSE_REQUEST'
  | 'INACTIVE'

/**
 * Every notification the service can send: the names in `Notice` in
 * `backend/src/domain/notification.rs`. The backend's tests fail if a
 * language is missing one or has one the service never sends.
 */
export type NotificationKind =
  | 'INVITATION_CLAIMED'
  | 'INVITATION_CLAIMED_UNCONFIRMED'
  | 'COUNTERPARTY_CONFIRMED'
  | 'CLAIMANT_LEFT'
  | 'REVISION_SENT'
  | 'REVISION_SENT_UNCONFIRMED'
  | 'AMENDMENT_PROPOSED'
  | 'ACCEPTANCE_WAITING'
  | 'AGREEMENT_IN_FORCE'
  | 'AMENDMENT_IN_FORCE'
  | 'AMENDMENT_DECLINED'
  | 'AMENDMENT_WITHDRAWN'
  | 'AMENDMENT_EXPIRED'
  | 'DELIVERY_CLAIMED'
  | 'CLAIM_RETRACTED'
  | 'DELIVERY_CONFIRMED'
  | 'DISPUTE_OPENED'
  | 'CONTRIBUTION_WAIVED'
  | 'END_PROPOSED'
  | 'END_PROPOSAL_CANCELLED'
  | 'CLOSE_REQUESTED'
  | 'CLOSE_REQUEST_RETRACTED'
  | 'STATEMENT_ADDED'
  | 'INACTIVITY_PROMPTED'
  | 'DUE_SOON'
  | 'OVERDUE_TO_DELIVER'
  | 'OVERDUE_TO_RECEIVE'
  | 'CLOSED_WITHDRAWN'
  | 'CLOSED_DECLINED'
  | 'CLOSED_EXPIRED'
  | 'CLOSED_COMPLETED'
  | 'CLOSED_ENDED_BY_AGREEMENT'
  | 'CLOSED_UNRESOLVED'
  | 'CLOSED_INACTIVE'

/**
 * Events worded without saying who caused them. Someone who opened the
 * invitation and left before being confirmed is one of them: the record
 * names the two parties, and that person was never one.
 */
export type NeutralEventType =
  | 'COUNTERPARTY_RELEASED'
  | 'REVISION_SUPERSEDED'
  | 'REVISION_EXPIRED'
  | 'AGREEMENT_IN_FORCE'
  | 'INACTIVITY_PROMPTED'

/**
 * Events that one of the parties did. Every event type the API can return is
 * one of these, a neutral one, or the exchange closing, so a new type fails
 * the typecheck until each language has words for it.
 */
export type PartyEventType = Exclude<Schemas['EventType'], NeutralEventType | 'EXCHANGE_CLOSED'>

/**
 * What someone can have done from the invited party's place before being
 * removed from it or leaving it (DESIGN.md §8): open the invitation, sign,
 * and, in an exchange older than the rule against it, send terms of their
 * own. These are worded without a name, since the name the agreement gives
 * that place is not theirs.
 */
export type FormerClaimantEventType = 'COUNTERPARTY_CLAIMED' | 'REVISION_SENT' | 'REVISION_ACCEPTED'

/**
 * What can happen to one contribution. Money is paid outside the product and
 * only recorded here, so these are worded a second time for money, in words
 * for paying and receiving rather than delivering (DESIGN.md §7, §11).
 */
export type ContributionEventType = Extract<Schemas['EventType'], `CONTRIBUTION_${string}`>

/** What a note written with an event is: a message, a reason, a statement, or just a note. */
export type RecordNoteKind = 'message' | 'note' | 'reason' | 'statement'

/**
 * Every piece of text the product says. Each language's file in `wording/`
 * must supply all of it, so a missing translation fails the build
 * (DESIGN.md §4.2). What users write themselves is never translated and does
 * not live here.
 *
 * Text that contains a number or a name is written as an ICU message, so
 * each language can order the words and choose its own plural forms. Never
 * build a sentence by joining pieces. `formatMessage` fills one in.
 */
export interface Wording {
  productName: string
  tagline: string
  /** What a messaging app shows for a shared link. Never names a person, a term or an amount. */
  linkPreview: {
    title: string
    description: string
  }
  service: {
    checking: string
    connected: string
    unreachable: string
    /** The web app's build is below the minimum the service accepts changes from. */
    outdatedWeb: string
    reload: string
  }
  /**
   * What the service sends when the other party does something, when an
   * agreement comes into force, and to remind a party that something is due
   * soon or overdue. The clients never show these; they live here so that
   * everything the product says is in one file per language. A message may
   * use `{code}` (the exchange's display code), `{productName}` and
   * `{recordLink}` (the exchange's record, to read, print or download), and
   * never carries anything from the agreement itself (DESIGN.md §12).
   *
   * `AGREEMENT_IN_FORCE` and `AMENDMENT_IN_FORCE` go to both signers and
   * must give `{recordLink}`: they are how each is told where their signed
   * copy is (DESIGN.md §14.1). A reminder may cover several contributions at
   * once and names none of them.
   */
  notifications: {
    email: {
      /** Wraps every message. Must contain `{body}` and `{link}`; may use `{code}` and `{productName}`. */
      layout: string
      messages: Record<NotificationKind, { subject: string; body: string }>
    }
    /**
     * The email that carries a one-time code, one message for each thing a
     * code can be asked for, so that the message says what the code does and
     * nobody is talked into reading out a "sign-in code" that would delete
     * their account. May use `{code}` and `{productName}`.
     */
    oneTimeCode: {
      signIn: { subject: string; body: string }
      deleteAccount: { subject: string; body: string }
    }
    /**
     * The email telling a reviewer that a report is waiting (DESIGN.md §9).
     * Says nothing about the report. Uses `{productName}` and `{link}`, the
     * review screen; a paragraph ending in `{link}` becomes its button.
     */
    staffAlert: { subject: string; body: string }
    /**
     * The email telling every address on two accounts that they were
     * combined (`backend/src/combine.rs`). Names no yup. Uses
     * `{productName}` and `{link}`, the account page.
     */
    accountsCombined: { subject: string; body: string }
    /**
     * The email telling an address that it was replaced on its account by
     * another, sent to the old one. Uses `{productName}` and `{link}`.
     */
    emailChanged: { subject: string; body: string }
    /**
     * The email telling an address that it was removed from its account,
     * sent to it. Uses `{productName}` and `{link}`.
     */
    emailRemoved: { subject: string; body: string }
  }
  /**
   * The push notification the service sends, for every notice alike
   * (DESIGN.md §12): a lock screen is read by whoever holds the phone, so it
   * names no exchange, party, term or amount. May use `{productName}`.
   */
  push: {
    body: string
  }
  /**
   * The service's own text messages, all of them "Yuppers.app agreement
   * updates". One-time codes by text are not among them: Twilio Verify
   * texts those in its own template (`backend/src/notifications/verify.rs`).
   */
  sms: {
    /**
     * An agreement update by text, sent by the service: `{link}` to the
     * exchange and nothing else, within the GSM alphabet
     * (`backend/src/notifications/sms_updates.rs`).
     */
    update: string
    /** The text confirming that updates were turned on, as the carriers ask. */
    optInConfirmation: string
  }
  common: {
    loading: string
    cancel: string
    tryAgain: string
    skipToContent: string
    notFoundTitle: string
    notFoundBody: string
    goHome: string
  }
  nav: {
    label: string
    exchanges: string
    account: string
    signOut: string
    language: string
  }
  signIn: {
    title: string
    intro: string
    identifierLabel: string
    identifierHint: string
    sendCode: string
    codeSent: string
    /**
     * Said under `codeSent` for a code sent by email: where to look for it.
     * `{sender}` is the address it comes from (`GET /v1/meta`,
     * `code_sender`); `checkInboxAnySender` where the service does not say.
     */
    checkInbox: string
    checkInboxAnySender: string
    /** Said until `resend` is offered, a while after a code was sent. */
    resendSoon: string
    codeLabel: string
    codeHint: string
    submit: string
    resend: string
    resent: string
    changeIdentifier: string
    /**
     * The same as `intro`, `identifierLabel` and `changeIdentifier` where
     * the service takes email addresses only (`GET /v1/meta`).
     */
    introEmail: string
    emailLabel: string
    changeEmail: string
    /**
     * `identifierHint`, naming the country codes served: `{codes}`, such as
     * `+1, +52`. A US number needs none; the hint asks for one only for
     * numbers elsewhere.
     */
    identifierHintCountries: string
    /** `identifierHint` where the service texts US numbers only (`+1`): written as they are in the US. */
    identifierHintUs: string
    /** Said, before anything is sent, of a phone number where only email addresses are taken. */
    emailOnly: string
    /** `INVALID_IDENTIFIER`, where only email addresses are taken. */
    invalidEmail: string
  }
  /**
   * The web app's note, on the sign-in form, for a page open in another
   * app's built-in browser, which may not keep the person signed in
   * (`apps/web/src/lib/in-app-browser.ts`).
   */
  inAppBrowser: {
    /** On iOS, where the way out is Safari. `{app}` is the app's name, or `someApp`. */
    noteIos: string
    /** Elsewhere, where it is the person's own browser. */
    noteOther: string
    /** `{app}` where the app does not say which it is. */
    someApp: string
    /** The button that opens the page in Safari (iOS 17 and later). */
    openSafari: string
    /** The button that opens the page in the default browser (Android). */
    openBrowser: string
    /** The other way out: the app's own menu, or the copied link. */
    menu: string
    copy: string
    copied: string
    copyFailed: string
    /** Hides the note until the page is loaded again. */
    hide: string
  }
  profile: {
    firstTitle: string
    firstIntro: string
    title: string
    nameLabel: string
    nameHint: string
    nameRequired: string
    languageLabel: string
    adultLabel: string
    adultConfirmed: string
    adultRequired: string
    detailLabel: string
    detailHint: string
    continue: string
    save: string
    saved: string
    emailLabel: string
    phoneLabel: string
    /**
     * Which build of the app this is, shown small at the foot of the account
     * screen (`versionText` in `build.ts`): the version and the commit it was
     * built from, and on the apps the store build number too.
     */
    version: string
    versionOnly: string
    versionBuild: string
    versionBuildCommit: string
  }
  /**
   * The account's email address and phone number, each with Add, Change and
   * Remove (`identifiers.ts`). `{identifier}` and `{staying}` are shown as
   * written, phone numbers formatted.
   */
  identifiers: {
    heading: string
    intro: string
    none: string
    add: string
    change: string
    remove: string
    addEmailTitle: string
    addPhoneTitle: string
    changeEmailTitle: string
    changePhoneTitle: string
    removeEmailTitle: string
    removePhoneTitle: string
    newEmailLabel: string
    newPhoneLabel: string
    changeNote: string
    /**
     * Changing one first takes a code to one of the account's own, the one
     * being replaced or the other (`POST /v1/me/identifiers/proof`).
     * `{identifier}` is where it goes.
     */
    proveIntro: string
    /** As `proveIntro`, for adding one where the account has none of that kind. */
    proveAddIntro: string
    proveOther: string
    proveSend: string
    proveConfirm: string
    /** The email address replaced is told; `{identifier}` is that one. */
    changeEmailTold: string
    sendCode: string
    codeSent: string
    confirm: string
    /** Why Remove is not offered for the only one. */
    onlyEmail: string
    onlyPhone: string
    removeIntro: string
    removePhoneNote: string
    removeEmailNote: string
    sendRemovalCode: string
    removeConfirm: string
    added: string
    removed: string
    /**
     * The notices shown once in the app for a phone number replaced or
     * removed, which is not texted (`notice` in `GET /v1/me`). Use `{date}`.
     */
    noticePhoneChanged: string
    noticePhoneRemoved: string
  }
  /**
   * The offer to combine another account into this one, once a code proved
   * its address (`IDENTIFIER_ON_OTHER_ACCOUNT`), and its confirmation.
   * `{identifier}` is masked.
   */
  combine: {
    headingEmail: string
    headingPhone: string
    intro: string
    otherHeading: string
    otherName: string
    otherEmail: string
    otherPhone: string
    /** Uses `{count}`, a plural. */
    yups: string
    /** Uses `{inForce}`, `{negotiating}`, `{drafts}`, `{closed}`. */
    yupsDetail: string
    movesHeading: string
    movesYups: string
    emailAdded: string
    emailReplaced: string
    emailDropped: string
    phoneAdded: string
    phoneReplaced: string
    phoneDropped: string
    paymentMove: string
    paymentDropped: string
    textUpdatesMove: string
    textUpdatesEnd: string
    devices: string
    ends: string
    cannotUndo: string
    expires: string
    confirm: string
    done: string
    /**
     * Shown once on the account another was combined into, where neither
     * had an email address to tell (`combined_notice`). Uses `{date}`.
     */
    noticeBanner: string
    noticeDismiss: string
  }
  /**
   * The appearance switch: System, Light or Dark, kept on this device only.
   * On the account screen, and on the web in the footer too, for someone
   * reading an invitation without an account.
   */
  appearance: {
    heading: string
    system: string
    light: string
    dark: string
    hint: string
  }
  /**
   * Deleting the account: what it does and does not delete, said before it
   * is done, the code that confirms it, and what the other party of an open
   * exchange is shown afterwards.
   */
  deletion: {
    heading: string
    open: string
    intro: string
    loadFailed: string
    deletedHeading: string
    deletedAccount: string
    deletedData: string
    signUpAgain: string
    exchangesHeading: string
    nothingOpen: string
    /** One sentence each, with `{count}`, for what the account is still in. */
    drafts: string
    openProposals: string
    agreementsInForce: string
    agreementsStand: string
    otherPartyTold: string
    keptHeading: string
    keptAgreements: string
    keptReports: string
    /** `{identifier}` is the account's own email address or phone number. */
    codeIntro: string
    codeChoice: string
    sendTo: string
    sendCode: string
    codeSent: string
    codeRequired: string
    continue: string
    confirmHeading: string
    confirmBody: string
    confirm: string
    back: string
    deleted: string
    dismiss: string
    /** Shown to the other party while the exchange is still open. `{name}` is who left. */
    otherPartyLeftActive: string
    otherPartyLeftNegotiating: string
  }
  home: {
    title: string
    start: string
    empty: string
    withParty: string
    noParty: string
    reference: string
    updated: string
    tooManyToday: string
    /** The list in groups: what is in progress first, then drafts, then what is closed, folded away. */
    groupOpen: string
    groupDrafts: string
    groupClosed: string
    showClosed: string
    hideClosed: string
    /**
     * On the initiator's card while nobody has joined through their link:
     * `notSent` while they have not opened a way to send it, `waitingFor`
     * with `{name}` once they have.
     */
    notSent: string
    waitingFor: string
  }
  states: Record<Schemas['StateDto'], string>
  outcomes: Record<Schemas['OutcomeDto'], string>
  closedReasons: Record<ClosedReason, string>
  party: {
    you: string
    other: string
    nameYou: string
  }
  contributionTypes: Record<Schemas['ContributionType'], string>
  contributionStatus: Record<Schemas['Status'], string>
  /** The same statuses for a money contribution, in words for paying and receiving. */
  moneyStatus: Record<Schemas['Status'], string>
  /** Labels around an agreement's terms. The terms themselves are the parties' own words. */
  terms: {
    ownWords: string
    partiesHeading: string
    termsHeading: string
    youProvide: string
    otherProvides: string
    nothing: string
    required: string
    optional: string
    quantity: string
    quantityWithUnit: string
    amount: string
    criteriaLabel: string
    dueOnAgreement: string
    dueOnDate: string
    /**
     * A due date shown to someone whose device keeps another time zone than
     * the exchange's; `zone` is the exchange's, by its city.
     */
    dueOnDateInZone: string
    dueAfter: string
    overdue: string
    timezone: string
    fingerprint: string
    /** On every money contribution: paid outside the product and only recorded here. */
    moneyOutside: string
  }
  composer: {
    titleFirst: string
    titleCounter: string
    titleAmend: string
    introFirst: string
    introCounter: string
    introAmend: string
    partiesLegend: string
    yourName: string
    otherName: string
    termsLabel: string
    termsHint: string
    itemsHeading: string
    itemLegend: string
    addYours: string
    addTheirs: string
    remove: string
    fromLabel: string
    typeLabel: string
    descriptionLabel: string
    amountLabel: string
    amountPreview: string
    quantityLabel: string
    unitLabel: string
    dueLabel: string
    dueOnAgreement: string
    dueOnDate: string
    dueAfter: string
    dateLabel: string
    /** Under the due date, when the writer's device keeps another time zone than the exchange's. */
    dateInZone: string
    afterLabel: string
    afterChoose: string
    itemOption: string
    itemOptionBlank: string
    criteriaLabel: string
    requiredLabel: string
    locked: string
    noteLabel: string
    noteHint: string
    saving: string
    saved: string
    saveFailed: string
    review: string
    problemsSummary: string
    /** One entry per thing `buildTerms` can find wrong with a working copy. */
    problems: Record<ProblemCode, string>
    signTitle: string
    signIntro: string
    yourNote: string
    backToEdit: string
    signAndSend: string
    conflict: string
    staleDraft: string
    staleDraftKeep: string
    staleDraftDiscard: string
    notAvailable: string
    /** Beside a money amount while it is written. */
    moneyOutside: string
    /** Throwing away a draft that was never sent, said in full before it is done. */
    discard: string
    discardText: string
    confirmDiscard: string
    /**
     * What an amendment will do to each item of the agreement, predicted from
     * the same rule the service applies (DESIGN.md §7).
     */
    effectsHeading: string
    effectsIntro: string
    effectsSteer: string
    effects: Record<AmendmentEffect, string>
    /** Uses `{count}`: items of the agreement the working copy no longer has. */
    removedCount: string
  }
  /**
   * What a signer is shown before signing. Versioned: `CONSENT_VERSION` names
   * the version these messages are, and changes whenever they do
   * (DESIGN.md §14.1).
   */
  consent: {
    heading: string
    pendingReview: string
    binding: string
    electronic: string
    noJudge: string
    agree: string
  }
  invitationLink: {
    intro: string
    shownOnce: string
    linkLabel: string
    copy: string
    copied: string
    copyFailed: string
    share: string
    /** Sent along with a shared link. Like the preview, never a name, a term or an amount. */
    shareText: string
    /**
     * `shareText` with the link in it, `{link}`, for the ways of sharing that
     * take one piece of text: an email's body, a text message, WhatsApp.
     */
    shareMessage: string
    /** The ways of sharing offered where the device has no share sheet of its own. */
    shareEmail: string
    shareSms: string
    shareWhatsApp: string
    shareQr: string
    hideQr: string
    /** The QR code's name for a screen reader, and what it is for. */
    qrLabel: string
    qrHint: string
    qrFailed: string
    unclaimed: string
    reissueIntro: string
    /**
     * Who the invitation is for (DESIGN.md §8), asked in the composer and when
     * a link is replaced. Naming them is expected: `forIntro` asks the
     * question and says why, `forHint` that they must sign in with exactly
     * what is given, `forNoContact` that we do not contact them.
     */
    forLabel: string
    forIntro: string
    forHint: string
    forNoContact: string
    /** `forLabel` where the service takes email addresses only (`GET /v1/meta`). */
    forLabelEmail: string
    /** Said when nobody is named and a link for anyone was not chosen; `…Email` as for the label. */
    forMissing: string
    forMissingEmail: string
    /**
     * Said, before anything is sent, of what `forLabel` was given: not an
     * email address or phone number at all; not an email address where only
     * those are taken; a phone number where only email addresses are taken;
     * a phone number of a country the service does not text (`{codes}`).
     */
    forInvalid: string
    forInvalidEmail: string
    forEmailOnly: string
    forCountry: string
    /**
     * The deliberate exception: a link for anyone. `forAnyone` chooses it,
     * `forAnyoneText` says what it costs once chosen, `forNamed` goes back
     * to naming them.
     */
    forAnyone: string
    forAnyoneText: string
    forNamed: string
    /** On the signing step, who the invitation is for: `{identifier}`, as typed. */
    boundSummary: string
    /** On the signing step, for a link for anyone. */
    forAnyoneSummary: string
    reissue: string
    /**
     * Sending the link, a step of its own after signing (DESIGN.md §8):
     * `sendTitle` heads it, `sendRule` says that Yuppers does not send it
     * and that `{name}` gets nothing until the person does. The primary
     * action depends on who the link is for: `sendText` opens a text
     * message to a phone number, `sendEmail` an email to an address,
     * `share` the device's share sheet for a link for anyone;
     * `sendWhatsApp` and `copy` stand beside them. `sharedNotice` follows
     * any of them, with `sendDone` to go on. `later` is the way past the
     * step without sending, and `laterHint` says what that leaves.
     */
    sendTitle: string
    sendRule: string
    sendText: string
    sendEmail: string
    sendWhatsApp: string
    sharedNotice: string
    sendDone: string
    later: string
    laterHint: string
    /**
     * The reminder on the exchange's page while nobody has joined:
     * `notJoined` heads it; `notSentYet` while no way to send the link was
     * ever opened, `sharedOn` with `{date}` once one was, `sharedLongAgo`
     * when that was long enough ago to ask again, and `sendAgain` where the
     * link is no longer at hand and a new one has to be made.
     */
    notJoined: string
    notSentYet: string
    sharedOn: string
    sharedLongAgo: string
    sendAgain: string
  }
  /**
   * The invitation page. Signed out it shows only `signedOutTitle` and
   * `signInToRead` above the way to sign in: the proposal is read signed in
   * (DESIGN.md §9), so nothing about the link shows before then.
   */
  invitation: {
    /** The page's heading once the proposal is shown. */
    title: string
    signedOutTitle: string
    signInToRead: string
    introSignedIn: string
    notBinding: string
    expires: string
    boundSignedIn: string
    noteHeading: string
    /** The button for an account that has not given its name yet. */
    respondNew: string
    respondAs: string
    opening: string
    missingTitle: string
    missing: string
    ownInvitation: string
    /**
     * An invitation sent to an address the account does not have
     * (`sent_to`): only its kind is said, never the address, in full or in
     * part. The person types it.
     */
    sentToEmail: string
    sentToPhone: string
    sentToReplacesEmail: string
    sentToReplacesPhone: string
    signInInstead: string
    addressCodeSent: string
    addAndOpen: string
    replaceAndOpen: string
  }
  /**
   * Someone who opened an invitation that named nobody, until the initiator
   * confirms them (DESIGN.md §8): what the initiator is told about removing
   * them, and what they are told about the little they can do until then.
   */
  claimant: {
    /** The initiator's other answer to "is this who you invited?". */
    reject: string
    rejectTitle: string
    /** Uses `{name}`: the claimant's own display name, which the initiator was shown. */
    rejectRemoves: string
    rejectVoids: string
    rejectKeeps: string
    rejectQuiet: string
    confirmReject: string
    /** Shown to the initiator once the claimant is gone. */
    rejected: string
    /** In place of "nobody has opened your link yet", when someone has and is gone. */
    linkUsed: string
    /** To the claimant: what they can and cannot do while unconfirmed. */
    limits: string
    leave: string
    /** Uses `{name}`: the initiator. */
    leaveText: string
    leaveVoids: string
    confirmLeave: string
    /** The invitation page's introduction when the invitation names nobody. Uses `{name}`. */
    invitationIntroSignedIn: string
    /** What blocking does for a claimant, in place of declining what is open. */
    blockLeaves: string
    /** On a blocked person's entry, when the exchange it names is one the reader has left. */
    blockedAfterLeaving: string
    /** On the record page. Uses `{date}`, when it was signed, and `{since}`, when it became void. */
    voidSignature: string
  }
  /** Reporting an exchange and blocking the other party (DESIGN.md §9). */
  safety: {
    heading: string
    report: string
    reportProposal: string
    reportIntro: string
    reportProposalIntro: string
    reasonLegend: string
    /** One entry per reason a report can give. */
    reasons: Record<Schemas['ReportReason'], string>
    reasonRequired: string
    detailsLabel: string
    detailsRequiredLabel: string
    detailsHint: string
    detailsRequired: string
    sendReport: string
    reportSent: string
    tooManyReports: string
    block: string
    /** What blocking does, said before it is done: one sentence each. */
    blockStops: string
    blockEnds: string
    blockKeeps: string
    /**
     * Said too when the block is made from an agreement in force: what the
     * blocked person can still do in it, and that it can be closed.
     */
    blockInForce: string
    blockThenClose: string
    blockQuiet: string
    confirmBlock: string
    blocked: string
    /**
     * Beside a block, on an agreement in force that nobody has asked to close
     * yet, before the button that asks to close it without agreement.
     */
    blockedInForce: string
    unblock: string
    unblocked: string
    blockedHeading: string
    blockedIntro: string
    blockedEmpty: string
    blockedSince: string
  }
  /**
   * The staff screen where reports are reviewed (DESIGN.md §9), on the web
   * only and for reviewers only. English only would be acceptable here; the
   * keys are checked like every other screen's.
   */
  staff: {
    title: string
    /** Uses `{hours}`. */
    intro: string
    queueHeading: string
    queueEmpty: string
    /** Uses `{code}`. */
    reportLink: string
    reportLinkNoYup: string
    /** Uses `{hours}`, a plural. */
    waiting: string
    /** Uses `{minutes}`, a plural. */
    waitingMinutes: string
    overdue: string
    reason: string
    details: string
    noDetails: string
    reporter: string
    /**
     * Who reported, for a report with no account behind it: one made through
     * an invitation link without signing in, before reporting needed an
     * account. No report made now reads this.
     */
    reporterLink: string
    subject: string
    /** Uses `{id}`. */
    account: string
    /** Uses `{name}`. */
    nameInYup: string
    standing: Record<Schemas['AccountStanding'], string>
    /** An account combined into another: uses `{id}` and `{date}`. */
    mergedInto: string
    /** Uses `{date}`. */
    filed: string
    back: string
    /** Uses `{code}`. */
    detailTitle: string
    decisionHeading: string
    decisionIntro: string
    /** The button for each decision. */
    outcomes: Record<Schemas['ReviewOutcome'], string>
    /** What each decision does, said before it is confirmed. */
    outcomeText: Record<Schemas['ReviewOutcome'], string>
    /** Said once it is done. */
    outcomeDone: Record<Schemas['ReviewOutcome'], string>
    /** How another report was resolved. */
    outcomeNames: Record<Schemas['ReviewOutcome'], string>
    noteLabel: string
    noteOptionalLabel: string
    noteRequired: string
    confirm: string
    recordHeading: string
    recordIncomplete: string
    noRecord: string
    contentHidden: string
    otherReportsHeading: string
    status: Record<Schemas['ReportStatus'], string>
    historyHeading: string
    historyEmpty: string
    actions: Record<Schemas['ReviewAction'], string>
    /** Uses `{date}` and `{id}`. */
    byReviewer: string
    /** Uses `{date}`. */
    byOwner: string
    suspensionsHeading: string
    suspensionsEmpty: string
    /** Uses `{date}`. */
    suspendedSince: string
    lift: string
    liftText: string
    lifted: string
    hiddenHeading: string
    hiddenEmpty: string
    /** Uses `{date}`. */
    hiddenSince: string
    restore: string
    restoreText: string
    restored: string
  }
  /**
   * What only the mobile app says. Everything else it shows is the wording the
   * web app uses for the same thing.
   */
  mobile: {
    back: string
    /** Opening an invitation from a link that was pasted, where the system did not hand it over. */
    openInvitation: {
      title: string
      intro: string
      label: string
      hint: string
      paste: string
      open: string
      invalid: string
    }
    /**
     * The record screen, where it differs from the web's page: there is no
     * printing, and the copy leaves through the system's share sheet.
     */
    record: {
      openHint: string
      share: string
      /** Said beside the share button: what the copy holds and who can then read it. */
      shareHint: string
      shareUnavailable: string
      shareFailed: string
      /** The record as a PDF, summary first, handed to the share sheet. */
      savePdf: string
      savePdfHint: string
      pdfFailed: string
    }
    /** On the signed-out first screen: the way to an invitation without an account. */
    invited: {
      heading: string
      introSignIn: string
    }
    /** Push notifications on this phone: the offer on the list, and the account screen's switch. */
    notifications: {
      heading: string
      switch: string
      switchHint: string
      /** When the system's settings refuse notifications to the app. */
      blocked: string
      openSettings: string
      failed: string
      askHeading: string
      askBody: string
      turnOn: string
      notNow: string
      /** The Android notification channel's name, shown in the system's settings. */
      channel: string
      /** To an account with no email address, where push can be had. */
      phoneOnly: string
      /** To an account with no email address, where it cannot. */
      phoneOnlyNoPush: string
    }
    date: {
      choose: string
      /** What a screen reader says the date button does. Uses `{date}`. */
      change: string
    }
  }
  /**
   * The guided way out of an agreement in force that isn't working
   * (DESIGN.md §5.3). Every way it offers is an existing action; `means`
   * says in one sentence who each one releases from what. `{name}` is the
   * other party.
   */
  trouble: {
    open: string
    intro: string
    question: string
    situations: Record<TroubleSituation, string>
    explain: Record<TroubleSituation, string>
    change: string
    means: Record<TroubleWay, string>
    nothingTheyOwe: string
    nothingInDoubt: string
    endPending: string
    closePending: string
    amendPending: string
  }
  /** Said where a dispute is opened or seen: the product records, and does not rule (DESIGN.md §14.1). */
  dispute: {
    weRecord: string
    pointer: string
  }
  /**
   * What a proposal changes, shown to the person asked to sign it: against
   * the agreement in force for an amendment, or the version it answers for
   * a counteroffer.
   */
  proposalChanges: {
    heading: string
    againstInForce: string
    againstPrevious: string
    nothing: string
    termsChanged: string
    kinds: Record<ItemChangeKind, string>
    /** Uses `{count}`. */
    unchangedCount: string
    fields: Record<ChangedField, string>
    /** One changed field in the product's words. Uses `{field}`, `{before}` and `{after}`. */
    fieldChange: string
    /** Before and after a changed field written in the parties' own words. */
    was: string
    now: string
    /** A field that had, or has, no value. */
    notSet: string
    /** Where an amended item would stand. Uses `{status}`. */
    statusAfter: string
  }
  exchange: {
    title: string
    titleNoName: string
    refresh: string
    updated: string
    newer: string
    showLatest: string
    claimedHeading: string
    claimedBody: string
    claimedSigned: string
    confirmCounterparty: string
    notThem: string
    waitingConfirmation: string
    waitingConfirmationSigned: string
    proposalHeading: string
    amendmentHeading: string
    version: string
    sentByYou: string
    sentByOther: string
    expires: string
    noteFromYou: string
    noteFromOther: string
    signedByYou: string
    signedByOther: string
    unsignedByYou: string
    unsignedByOther: string
    accept: string
    decline: string
    counter: string
    withdraw: string
    change: string
    acceptBlocked: string
    declineEnds: string
    declineKeeps: string
    withdrawEnds: string
    withdrawKeeps: string
    confirmDecline: string
    confirmWithdraw: string
    signHeading: string
    signIntro: string
    agreementHeading: string
    agreementSigned: string
    remaining: string
    amend: string
    /** What each fulfillment action is called, and what the person is told before doing it. */
    moves: Record<Move, string>
    moveText: Record<Move, string>
    /**
     * The same for money, which is paid outside the product and only recorded
     * here: words for paying and receiving, not delivering.
     */
    moneyMoves: Record<Move, string>
    moneyMoveText: Record<Move, string>
    /**
     * To the provider of something marked delivered, or paid, that the other
     * party has left unconfirmed for a long time: the ways out. Uses `{name}`
     * and `{date}`.
     */
    waitingLong: string
    waitingLongMoney: string
    /** Under a close request: when it lapses. Uses `{date}`. */
    closeRequestLapses: string
    /** A reviewer has hidden what the parties wrote from the reader (DESIGN.md §9). */
    contentHidden: string
    noteLabel: string
    reasonLabel: string
    remedyLabel: string
    noteRecord: string
    noteRequired: string
    endingHeading: string
    proposeEnd: string
    proposeEndText: string
    sendEndProposal: string
    endProposedByYou: string
    endProposedByOther: string
    cancelEnd: string
    declineEnd: string
    agreeEnd: string
    agreeEndText: string
    confirmAgreeEnd: string
    requestClose: string
    requestCloseText: string
    statementLabel: string
    statementRequiredLabel: string
    sendCloseRequest: string
    closeRequestedByYou: string
    closeRequestedByOther: string
    retractClose: string
    addStatement: string
    sendStatement: string
    statementAdded: string
  }
  /**
   * Text updates for an agreement (DESIGN.md §12), on the exchange view:
   * adding a phone number, the consent box, and where things stand.
   */
  /**
   * Payment options (`payments.ts`; `backend/src/payments.rs`): the names a
   * person may save for being paid in Venmo, Cash App, PayPal or Zelle, on
   * the account screen; showing them on one yup; and what the person who
   * owes money sees. Yuppers never moves money: none of this may say or
   * suggest that a payment is protected, guaranteed, held or checked, nor
   * advise which kind of payment to choose in an app. App names are plain
   * text, never logos.
   */
  payments: {
    /**
     * The payment options screen, and its row on the account screen: the
     * title of both, and what they are for (shown with none added).
     */
    heading: string
    intro: string
    venmoLabel: string
    venmoHint: string
    venmoInvalid: string
    cashAppLabel: string
    cashAppHint: string
    cashAppInvalid: string
    paypalLabel: string
    paypalHint: string
    paypalInvalid: string
    zelleLabel: string
    zelleHint: string
    zelleInvalid: string
    /** Each app's name, in plain text: in the list, the summary and `{app}` below. */
    apps: {
      venmo: string
      cash_app: string
      paypal: string
      zelle: string
    }
    /** The account row's summary with none added; otherwise the apps' names. */
    noneAdded: string
    /** The screen with none added, above `intro`. */
    empty: string
    /** The button that starts adding one. */
    add: string
    /** In its place once every app has one. */
    allAdded: string
    /** Choosing which app to add, from those not added yet. */
    pickHeading: string
    pickIntro: string
    /** The one field's step. `{app}`. */
    addHeading: string
    editHeading: string
    /** Each row's buttons, and their accessible names with `{app}`. */
    edit: string
    editWhat: string
    removeOne: string
    removeWhat: string
    saveOne: string
    /** Once done. `{app}`. */
    savedOne: string
    removedOne: string
    /** Once the last one is removed: it is shown on no yup now. `{app}`. */
    removedLast: string
    /** The confirmation before removing one. `{app}`. */
    confirmTitle: string
    confirmText: string
    /** Instead of `confirmText` for the last one: it turns showing them off everywhere. */
    confirmLast: string
    /** Not removing it after all. */
    keep: string
    /** The web screen's way back to the account. */
    back: string
    /** On a yup, for a party who receives money. */
    showHeading: string
    showLabel: string
    /** Under the box. `{name}`: the other party. */
    showHint: string
    /** Once turned on or off. `{name}`. */
    shownNow: string
    hiddenNow: string
    /** With nothing saved: the link to the account screen. */
    noneSaved: string
    addInAccount: string
    /** With some saved: the link to the payment options screen, under the box. */
    manage: string
    /** In the panel where terms are signed or sent. */
    showWhenSigning: string
    /** For the payer. `{name}`, `{amount}`. */
    payButton: string
    sheetTitle: string
    /** Who the options come from, and what we don't do. `{name}`. */
    addedBy: string
    /** Neutral: no advice on which kind of payment to pick in the app. */
    appTerms: string
    /** Each app's button: its name in plain text. */
    open: {
      venmo: string
      cash_app: string
      paypal: string
    }
    /** How the person is shown in each app, under its button. `{handle}`. */
    handle: {
      venmo: string
      cash_app: string
      paypal: string
    }
    /** What opening it does. `{amount}`. */
    prefillsAmountAndNote: string
    prefillsAmount: string
    enterAmount: string
    /**
     * Beside an option the payee changed after the agreement came into
     * force, however long ago (`theirs_changed`, `backend/src/payments.rs`):
     * `{name}`, `{date}` it changed. Never the old value.
     */
    changed: {
      venmo: string
      cash_app: string
      paypal: string
      zelle: string
    }
    zelleHeading: string
    zelleNoLinks: string
    amountLabel: string
    noteLabel: string
    /** The note for the payment: `{title}` of the item, `{code}` of the yup. */
    note: string
    copy: string
    /** Its accessible name: `{what}` is the label of what is copied. */
    copyWhat: string
    copied: string
    copyFailed: string
    /** After paying: the existing claim, which only the payer makes. `{name}`. */
    afterHeading: string
    afterText: string
    close: string
    /** The payee stopped showing them while the sheet was being opened. `{name}`. */
    gone: string
  }
  smsUpdates: {
    heading: string
    intro: string
    addPhoneIntro: string
    phoneLabel: string
    phoneHint: string
    phoneInvalid: string
    sendCode: string
    /** `{phone}`, masked. */
    codeSent: string
    codeLabel: string
    /**
     * Adding a number takes a code to the account's email address too:
     * where it went (`{email}`), and its field.
     */
    proofCodeSent: string
    proofCodeLabel: string
    addPhone: string
    changePhone: string
    /** `{phone}`, masked. */
    phoneAdded: string
    /**
     * The consent wording beside the box, word for word as the terms quote
     * it; its two addresses are links. Its version is `SMS_CONSENT_VERSION`.
     */
    consent: string
    save: string
    /** `{phone}`, masked. */
    on: string
    off: string
    /** `{phone}`, masked. */
    optedOut: string
    /** The link to the page on how people opt in, and to the terms on texts. */
    howItWorks: string
  }
  /**
   * Consent to a one-time code by text (`sms-code-consent.ts`): the words
   * beside the box on each form that texts a code, word for word as the
   * terms quote them, their two addresses links. Their version is
   * `SMS_CODE_CONSENT_VERSION`.
   */
  smsCode: {
    /** On the sign-in form, once what is typed reads as a phone number. */
    signIn: string
    /** On account deletion, with the code to go to the phone number. */
    deleteAccount: string
    /** In "Text updates", adding a number to the account. */
    verifyNumber: string
    /** Beside the button that sends the code, while the box is not ticked: why it waits. */
    tickToSend: string
  }
  /**
   * Wallet passes (DESIGN.md §11): the button on the exchange view, and what
   * the service writes on a pass (`backend/src/wallet`). A pass carries
   * nothing from the agreement, so none of this takes a name or an amount.
   */
  wallet: {
    /** Above the button: what a pass is and is not. `{productName}`. */
    intro: string
    addToApple: string
    addToGoogle: string
    /** The button while the pass is being fetched. */
    adding: string
    /** The pass's labels and its fixed text. */
    pass: {
      /** What the pass is, for screen readers. `{productName}`, `{code}`. */
      description: string
      status: string
      reference: string
      nextDue: string
      outstanding: string
      with: string
      closedOn: string
      open: string
      /** On the back. `{productName}`. */
      note: string
      /** On the back of a revoked pass. */
      void: string
    }
    /** How the agreement stands, the pass's largest field. */
    status: {
      inForce: string
      waitingForYou: string
      dueSoon: string
      overdue: string
      disputed: string
      completed: string
      ended: string
      closed: string
      void: string
    }
    /** A date on a pass: `{month}`, `{day}`, `{year}`. */
    date: string
    months: {
      jan: string
      feb: string
      mar: string
      apr: string
      may: string
      jun: string
      jul: string
      aug: string
      sep: string
      oct: string
      nov: string
      dec: string
    }
  }
  /**
   * The record of an exchange: its history in the exchange view, the record
   * page, and what the downloaded copy says about itself.
   */
  record: {
    historyHeading: string
    historyEmpty: string
    /** The way to read the page of history before the one shown. */
    historyEarlier: string
    open: string
    openHint: string
    title: string
    back: string
    /** A reviewer has hidden what the parties wrote from the reader (DESIGN.md §9). */
    contentHidden: string
    print: string
    download: string
    /** The downloaded copy's file name, without its extension. */
    fileName: string
    madeFor: string
    timesIn: string
    summaryHeading: string
    started: string
    closedOn: string
    agreementIs: string
    agreementNone: string
    waiting: string
    endProposed: string
    closeRequested: string
    /** As `closedReasons`, for a page that may be read by neither party. */
    closedReasons: Record<ClosedReason, string>
    aboutHeading: string
    itemsHeading: string
    itemFrom: string
    since: string
    signaturesHeading: string
    verifiedBy: string
    verifiedAt: string
    consentShown: string
    versionsNone: string
    versionHeading: string
    versionSent: string
    versionAnswers: string
    versionStatus: Record<Schemas['RevisionStatus'], string>
    versionReplacedBy: string
    versionSignedBy: string
    eventsHeading: string
    /** What a note written with an event is called. */
    noteLabels: Record<RecordNoteKind, string>
    /**
     * One sentence per thing that can happen. `you` speaks to the party who
     * did it and `named` names them, with `{name}`; an event about a revision
     * may use `{number}`. An event about a contribution ends where the
     * contribution's description, the parties' own words, is shown after it.
     * `formerClaimant` is for what was done by someone since removed from
     * the invited party's place, and names nobody.
     */
    events: {
      you: Record<PartyEventType, string>
      named: Record<PartyEventType, string>
      /** The contribution events again, for a money contribution. */
      moneyYou: Record<ContributionEventType, string>
      moneyNamed: Record<ContributionEventType, string>
      formerClaimant: Record<FormerClaimantEventType, string>
      neutral: Record<NeutralEventType, string>
      closed: Record<Schemas['OutcomeDto'], string>
    }
    /**
     * What the service writes into a downloaded copy, and the record page
     * shows: what a signature rests on, that what the parties recorded about
     * delivery is theirs alone, and how to recompute a fingerprint
     * (DESIGN.md §14, §14.1). Plain text: the service puts these in as they
     * are, so they take no variables.
     */
    export: {
      about: string
      signatures: string
      statements: string
      contentHash: string
      verification: Record<Schemas['VerificationMethod'], string>
      /** What stands in place of text a reviewer has hidden from the reader. */
      hidden: string
    }
    /**
     * The plain summary at the top of the record (DESIGN.md §14.1): who,
     * what each gives, who signed and when, how it stands or ended, and what
     * became of each item. Everyone is named, as on the rest of the record.
     */
    summary: {
      heading: string
      intro: string
      /** Uses `{a}` and `{b}`. */
      between: string
      givesAgreed: string
      givesLast: string
      basisAgreement: string
      basisLast: string
      basisNone: string
      signedBy: string
      notSignedBy: string
      inForceFrom: string
      /** By state, or by outcome once closed. A closed one uses `{date}`. */
      standing: Record<'DRAFT' | 'NEGOTIATING' | 'ACTIVE' | Schemas['OutcomeDto'], string>
      endedProposedBy: string
      /** Uses `{name}`, `{provider}` and `{other}` as each needs. */
      outcome: Record<SummaryOutcome, string>
      moneyOutcome: Record<SummaryOutcome, string>
      savePdf: string
      savePdfHint: string
    }
  }
  /**
   * The ways to the help pages from the rest of the product. The pages' own
   * text is not here but in `wording/help/` (`HelpWording`), so that only
   * the help pages load it.
   */
  help: {
    /** The link to the help pages: the web app's footer, the mobile account screen. */
    link: string
    /** Said after a link that opens in a new browser tab, to screen readers only. */
    newTab: string
    /** Hint on a mobile link to help: it leaves the app for the browser. */
    inBrowser: string
    /** The link to the topic that explains one place in the product, named for that place. */
    learnMore: Record<HelpLinkPlace, string>
  }
  /**
   * The ways to the privacy policy. Its own text is not here but in
   * `wording/privacy/` (`PrivacyWording`), so that only its pages load it.
   */
  privacy: {
    /** The short link: the web app's footer, beside "Help". */
    link: string
    /** The policy's name, for a link that stands on its own: sign-in, the account screens, the help pages. */
    policy: string
    /** On the sign-in form, only where codes can go to phone numbers: the link to the policy's section on text messages. */
    smsLink: string
    /** Beside account deletion: the link to the policy's section on what deleting leaves. */
    deletion: string
  }
  /**
   * The ways to the terms and conditions. Their own text is in
   * `wording/terms/` (`TermsWording`), like the privacy policy's.
   */
  termsOfUse: {
    /** The short link: the web app's footer, beside "Privacy". */
    link: string
    /** The document's name, for a link that stands on its own, beside the privacy policy's. */
    document: string
  }
  /** One entry per error code the API can return. */
  errors: Record<ErrorCode, string>
  /**
   * Said only to assistive technology: hints a screen reader reads after a
   * control's label, and announcements of changes that are otherwise only
   * seen. Never shown as text on a screen.
   */
  a11y: {
    /** Hint on an exchange in the list, which opens it. */
    openExchange: string
    /** Hint on a field that has to be filled in, where the platform has no "required" state. */
    required: string
    /** Why the signing button cannot be pressed yet. */
    signNeedsAgreement: string
    /** Announced when earlier history has been read in. */
    earlierAdded: string
  }
}

/**
 * One piece of a help page, in order: a heading, which starts a section, a
 * paragraph, or a list. Every language's page has the same pieces in the
 * same order, which the wording check enforces.
 */
export type HelpBlock = { h: string } | { p: string } | { ul: string[] } | { ol: string[] }

export interface HelpTopicWording {
  title: string
  /** One sentence, under the topic's name in the list of topics and under its heading. */
  summary: string
  blocks: HelpBlock[]
}

/**
 * The help pages' own text, one file per language in `wording/help/`. A
 * message may use the figures in `HELP_FIGURES` as placeholders, such as
 * `{closeDays}`, and nothing else. Like all wording it is the product
 * speaking: it says what the product does today, never what is planned.
 */
export interface HelpWording {
  title: string
  intro: string
  /** Heading over the list of topics, on the first page and under each topic. */
  topicsHeading: string
  /** Heading over the list of a topic's own sections. */
  onThisPage: string
  /** The link back to the first page of help. */
  allTopics: string
  topics: Record<HelpTopic, HelpTopicWording>
}
