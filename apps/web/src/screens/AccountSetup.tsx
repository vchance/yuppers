import { isComplete, useI18n, useSession } from '../app/context'
import { BrandHero } from '../components/Brand'
import { PageHeading, StepHeading } from '../components/ui'
import { ProfileForm } from './ProfileForm'
import { SignIn } from './SignIn'

/**
 * Everything between "not signed in" and "ready to act": signing in with a
 * one-time code, then, for a new account, the profile. It shows whichever
 * step is next and nothing once both are done, so it can stand in for any
 * page that needs an account.
 *
 * As a page of its own it opens with the brand (`BrandHero`): the first
 * thing someone sees of the product should look like the product. Below an
 * invitation, which has a header of its own, the steps open plain.
 */
export default function AccountSetup({ headingLevel = 'h1' }: { headingLevel?: 'h1' | 'h2' }) {
  const { wording } = useI18n()
  const { account } = useSession()
  const standalone = headingLevel === 'h1'
  // Below the invitation, the step opens where the button that asked for it
  // was, and the keyboard is taken to it.
  const heading = (text: string) =>
    standalone ? (
      <PageHeading key={text}>{text}</PageHeading>
    ) : (
      <StepHeading key={text}>{text}</StepHeading>
    )

  if (!account) {
    return (
      <section>
        {standalone && <BrandHero />}
        {heading(wording.signIn.title)}
        <SignIn />
      </section>
    )
  }
  if (!isComplete(account)) {
    return (
      <section>
        {standalone && <BrandHero />}
        {heading(wording.profile.firstTitle)}
        <p>{wording.profile.firstIntro}</p>
        <ProfileForm account={account} first />
      </section>
    )
  }
  return null
}
