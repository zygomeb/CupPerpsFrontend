use scrypto::prelude::*;

#[blueprint]
mod prop_perps {
    // This implementation has set out to explore the idea
    // of Cup Perps, and using lp tokens instead of state
    // and dis/advantages of both
    struct CupPerp {
        pair: String, // eg. BTC/USD, for UX purposes only

        leverage: Decimal,
        funding_coeff: Decimal,

        long_cup: Vault,
        // needed to query, all endpoints squash same-resource vaults into one
        long_cup_value: Decimal, 
        short_cup: Vault,
        short_cup_value: Decimal,
        long_cup_lp: Decimal,
        short_cup_lp: Decimal,
        // unused, for testing purposes
        last_update: u64,
        // serious version: set pecision on int
        last_exrate: Decimal,

        // toy version has the oracle be a variable
        // able to be updated with set_oracle(n)
        oracle_update: u64,
        oracle_exrate: Decimal,

        long_lp_badge: Vault,
        long_lp_resource: ResourceAddress,
        short_lp_badge: Vault,
        short_lp_resource: ResourceAddress,
        stable_coin: ResourceAddress
    }

    impl CupPerp {

        pub fn instantiate_pair(pair: String, exch: Decimal, mut deposit: Bucket) -> ComponentAddress {
            // initial deposit doesn't return lp tokens
            // to ensure no division by 0
            debug!("INSTANTIATING PAIR");
            let long_lp_badge: Vault = Vault::with_bucket(ResourceBuilder::new_fungible()
                .divisibility(DIVISIBILITY_NONE)
                .mint_initial_supply(1));

            let short_lp_badge: Vault = Vault::with_bucket(ResourceBuilder::new_fungible()
                .divisibility(DIVISIBILITY_NONE)
                .mint_initial_supply(1));

            let mut name = "CupPerp Long ".to_owned();
            name.push_str(&*pair);
            // reminder to self: don't ever
            // set custom divisibility (other than none)
            let long_lp_resource = ResourceBuilder::new_fungible()
                .mintable(rule!(require(long_lp_badge.resource_address())), LOCKED)
                .burnable(rule!(require(long_lp_badge.resource_address())), LOCKED)
                .metadata("name", name)
                // doesn't work, TODO lookup what the wallet wants
                .metadata("icon", "https://www.iconsdb.com/icons/preview/green/up-xxl.png")
                .create_with_no_initial_supply();

            name = "CupPerp Short ".to_owned();
            name.push_str(&*pair);
            let short_lp_resource = ResourceBuilder::new_fungible()
                .mintable(rule!(require(short_lp_badge.resource_address())), LOCKED)
                .burnable(rule!(require(short_lp_badge.resource_address())), LOCKED)
                .metadata("name", name)
                .metadata("icon", "https://www.iconsdb.com/icons/preview/red/down-xxl.png")
                .create_with_no_initial_supply();

            let stable_coin = deposit.resource_address();

            let amm = deposit.amount().clone()/2;
            let long_cup = Vault::with_bucket(deposit.take(deposit.amount()/2));
            let short_cup = Vault::with_bucket(deposit);
            let long_cup_lp = dec!(1000);
            let short_cup_lp = dec!(1000);

            Self {
                pair: pair,

                leverage: dec!(5),
                funding_coeff: dec!("0.75"),

                long_cup: long_cup,
                long_cup_value: amm,
                short_cup: short_cup,
                short_cup_value: amm,
                long_cup_lp: long_cup_lp,
                short_cup_lp: short_cup_lp,
                last_update: 0,
                last_exrate: exch,

                oracle_update: 0,
                oracle_exrate: exch,

                long_lp_badge: long_lp_badge,
                long_lp_resource: long_lp_resource,
                short_lp_badge: short_lp_badge,
                short_lp_resource: short_lp_resource,

                stable_coin: stable_coin
            }
            .instantiate()
            .globalize()
        }

        pub fn update(&mut self) {
            // assert!(oracle-update >= last_update)
            assert!(self.oracle_exrate > dec!(0));

            if self.last_exrate == self.oracle_exrate {
                return
            }            

            let ONE: Decimal = dec!(1);
            let ZERO: Decimal = dec!(0);

            let delta = (self.oracle_exrate / self.last_exrate - ONE) * self.leverage;
            let long_d = delta * self.long_cup.amount();
            let short_d = delta * self.short_cup.amount();
            let ratio = self.long_cup.amount() / self.short_cup.amount();

            let funding = self.funding_coeff * if ratio > ONE { 
                    ONE/ratio
                } else { 
                    ratio 
                };

            // funding rebate always goes to the smaller cup
            let adj_delta = if long_d.abs() > short_d.abs() { 
                    (if delta > ZERO { funding } else { ONE / funding }) * short_d.abs()
                } else { 
                    (if delta > ZERO { ONE / funding } else { funding }) * long_d.abs() 
                };

            if delta > ZERO {
                self.long_cup.put(self.short_cup.take(adj_delta));
            } else {
                self.short_cup.put(self.long_cup.take(adj_delta));
            }

            self.long_cup_value = self.long_cup.amount();
            self.short_cup_value = self.short_cup.amount();

            self.last_exrate = self.oracle_exrate;
            self.last_update = self.oracle_update;
        }

        pub fn deposit(&mut self, side: bool, funds: Bucket) -> Bucket {
            self.update();

            let lp_caller; let lp; let cup;
            let lp_badge; let lp_resource;
            if side {
                lp = &mut self.long_cup_lp;
                cup = &mut self.long_cup;
                lp_badge = &self.long_lp_badge;
                lp_resource = self.long_lp_resource;
            } else {
                lp = &mut self.short_cup_lp;
                cup = &mut self.short_cup;
                lp_badge = &self.short_lp_badge;
                lp_resource = self.short_lp_resource;
            }

            lp_caller = 
                *lp * ((cup.amount() + funds.amount()) / cup.amount() - 1);
            *lp += lp_caller;
            (*cup).put(funds);

            self.long_cup_value = self.long_cup.amount();
            self.short_cup_value = self.short_cup.amount();

            return lp_badge.authorize(|| 
                borrow_resource_manager!(lp_resource).mint(lp_caller));  
        }

        pub fn withdraw(&mut self, side: bool, funds: Bucket) -> Bucket {
            self.update();

            let payout; let lp; let cup;
            let lp_badge; let lp_resource;
            if side {
                lp = &mut self.long_cup_lp;
                cup = &mut self.long_cup;
                lp_badge = &self.long_lp_badge;
                lp_resource = self.long_lp_resource;
            } else {
                lp = &mut self.short_cup_lp;
                cup = &mut self.short_cup;
                lp_badge = &self.short_lp_badge;
                lp_resource = self.short_lp_resource;
            }

            assert!(funds.resource_address() == lp_resource);
            payout = funds.amount() / *lp * cup.amount();
            *lp -= funds.amount();
            lp_badge.authorize(|| funds.burn());

            if side {
                self.long_cup_value = cup.amount() - payout;
            } else {
                self.short_cup_value = cup.amount() - payout;
            }

            return (*cup).take(payout);
        }

        pub fn show_cups(&self) -> (Decimal, Decimal) {
            (self.long_cup.amount(), self.long_cup.amount())
        }

        // unsure if it's better to run a getter 
        // or pull the data manuall out of the component
        // probably the second
        pub fn get_lp_resource_addr(&self) 
            -> (ResourceAddress, ResourceAddress) {
            return (
                self.long_lp_resource, 
                self.short_lp_resource)
        }

        pub fn value(&self, long: Decimal, short: Decimal) 
            -> Decimal {
            
            return 
                long / self.long_cup_lp 
                    * self.long_cup.amount()
              + short / self.short_cup_lp 
                    * self.short_cup.amount()
        }

        pub fn set_oracle(&mut self, n: Decimal) {
            self.oracle_exrate = n;
        }
    }
}